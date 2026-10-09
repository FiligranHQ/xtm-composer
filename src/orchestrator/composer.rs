use crate::api::{ApiConnector, ComposerApi, ConnectorStatus, RequestedStatus};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Detects a connector **name collision**: the container fetched by name for this
/// connector carries an `opencti-connector-id` label that belongs to a *different*
/// connector. This happens when two connectors share the same name (which OpenCTI is
/// meant to forbid) — the orchestrator looks a deployment up by name and returns the
/// wrong one, later surfacing as an obscure Kubernetes
/// "`selector` does not match template `labels`" error.
///
/// A container without the label is a collision too: the composer has always
/// labeled what it deploys, so it was created by someone else (e.g. a connector
/// deployed by hand) and must not be reconciled, let alone refreshed.
///
/// Returns the mismatched connector-id found on the container (or `<unlabeled>`)
/// when a collision is present, or `None` when the ids match.
fn detect_connector_id_mismatch(
    container: &OrchestratorContainer,
    connector_id: &str,
) -> Option<String> {
    match container.labels.get("opencti-connector-id") {
        Some(found) if found != connector_id => Some(found.clone()),
        Some(_) => None,
        None => Some("<unlabeled>".to_string()),
    }
}

/// Returns true when logs are due for this connector, and marks them as pushed.
/// Each connector has its own schedule: a single shared timer would let only the
/// first connector of each cycle report its logs. A connector seen for the first
/// time reports after one full schedule, as at startup.
fn logs_due(log_ticks: &mut HashMap<String, Instant>, connector_id: &str, schedule: Duration) -> bool {
    let now = Instant::now();
    let last = log_ticks.entry(connector_id.to_string()).or_insert(now);
    if now.duration_since(*last) >= schedule {
        *last = now;
        true
    } else {
        false
    }
}

async fn orchestrate_missing(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
) {
    // Connector is not provisioned, deploy the images
    let id = connector.id.clone();
    info!(id = id, "Deploying the container");
    let deploy_action = orchestrator.deploy(connector).await;
    match deploy_action {
        // Update the connector status
        Some(_) => {
            api.patch_status(id, ConnectorStatus::Stopped).await;
        }
        None => {
            warn!(id = id, "Deployment canceled");
        }
    }
}

async fn orchestrate_existing(
    log_ticks: &mut HashMap<String, Instant>,
    health_tick: &mut Instant,
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
    container: OrchestratorContainer,
) {
    // Connector is provisioned
    let connector_id = connector.id.clone();
    let current_status_fetch = connector.current_status.clone().unwrap_or("stopped".into()); // Default current to created
    let connector_status = ConnectorStatus::from_str(current_status_fetch.as_str()).unwrap();
    let requested_status_fetch = connector.requested_status.clone();
    let container_status = orchestrator.state_converter(&container);
    // Check for reboot loop and send health metrics
    let is_in_reboot_loop = container.is_in_reboot_loop();
    let final_status = if is_in_reboot_loop {
        warn!(
            id = connector_id,
            restart_count = container.restart_count,
            "Reboot loop detected"
        );
        // For now, we still report it as Started but with a warning log
        // In the future, we could add a new status like ConnectorStatus::Critical
        container_status
    } else {
        container_status
    };
    
    // Update the connector status if needed
    let container_status_not_aligned = final_status != connector_status;
    
    // Detect if connector just started
    let just_started = container_status_not_aligned && 
                       final_status == ConnectorStatus::Started && 
                       connector_status == ConnectorStatus::Stopped;
    
    // Send health metrics if:
    // - Connector just started (immediate reporting)
    // - OR connector is running and 30 seconds have elapsed
    let now = Instant::now();
    let should_send_health = just_started || 
        (final_status == ConnectorStatus::Started && 
         now.duration_since(health_tick.clone()) >= Duration::from_secs(30));
    
    if should_send_health {
        if let Some(started_at) = &container.started_at {
            info!(id = connector_id, "Reporting health metrics");
            api.patch_health(
                connector_id.clone(),
                container.restart_count,
                started_at.clone(),
                is_in_reboot_loop,
            ).await;
        }
        // Reset timer only for running connectors
        if final_status == ConnectorStatus::Started {
            *health_tick = now;
        }
    }
    if container_status_not_aligned {
        api.patch_status(connector.id.clone(), final_status)
            .await;
        info!(id = connector_id, "Patch status");
    }
    // In case of platform upgrade, we need to align all deployed connectors
    let requested_connector_hash = connector.contract_hash.clone();
    let current_container_hash = container.extract_opencti_hash();
    if !requested_connector_hash.eq(current_container_hash) {
        // Versions are not aligned
        info!(
            id = connector_id,
            hash = requested_connector_hash,
            "Refreshing"
        );
        orchestrator.refresh(connector).await;
    }
    // Align existing and requested status
    let requested_status = RequestedStatus::from_str(requested_status_fetch.as_str()).unwrap();
    match (requested_status, container_status) {
        (RequestedStatus::Stopping, ConnectorStatus::Started) => {
            info!(id = connector_id, "Stopping");
            orchestrator.stop(&container, connector).await;
        }
        (RequestedStatus::Starting, ConnectorStatus::Stopped) => {
            info!(id = connector_id, "Starting");
            orchestrator.start(&container, connector).await;
        }
        _ => {
            info!(id = connector_id, "Nothing to execute");
        }
    }
    // Get latest logs and update the platform on the logs schedule
    if logs_due(log_ticks, &connector_id, api.post_logs_schedule()) {
        let connector_logs = orchestrator.logs(&container, connector).await;
        match connector_logs {
            Some(logs) => {
                info!(id = connector_id, "Reporting logs");
                api.patch_logs(connector_id, logs).await;
            }
            None => {
                // No logs
            }
        }
    }
}

pub async fn orchestrate(
    log_ticks: &mut HashMap<String, Instant>,
    health_tick: &mut Instant,
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
) {
    // Get the current definition from OpenCTI
    let connectors_response = api.connectors().await;
    if connectors_response.is_some() {
        // First round trip to instantiate and control if needed
        let connectors = connectors_response.unwrap();
        // Iter on each definition and check alignment between the status and the container
        for connector in &connectors {
            // Get current containers in the orchestrator
            let container_get = orchestrator.get(connector).await;
            match container_get {
                Some(container) => {
                    // Surface connector name collisions clearly: if the container we
                    // got back by name belongs to a different connector-id, two
                    // connectors likely share the same name (OpenCTI should forbid
                    // this). Without this, the only symptom is an obscure Kubernetes
                    // selector-mismatch error.
                    // The deployment belongs to another connector, so don't reconcile
                    // it (refresh/start/stop) for this one. If it is an orphan of this
                    // manager, the cleanup below removes it and the connector gets
                    // deployed on the next cycle; otherwise (another manager's, or
                    // unlabeled, e.g. deployed by hand) it is left untouched.
                    if let Some(found_id) =
                        detect_connector_id_mismatch(&container, &connector.id)
                    {
                        error!(
                            name = connector.name,
                            expected_id = connector.id,
                            found_id = found_id,
                            "Connector name collision detected: an existing deployment with this name belongs to a different connector id (duplicate connector name?), skipping"
                        );
                        // The connector is stuck until the clash is resolved: tell the
                        // platform, not only the manager logs.
                        if logs_due(log_ticks, &connector.id, api.post_logs_schedule()) {
                            let message = format!(
                                "[XTM Composer] Connector not deployed: a deployment named \"{}\" already exists and belongs to another owner ({}). Rename this connector or remove that deployment.",
                                connector.container_name(),
                                found_id
                            );
                            api.patch_logs(connector.id.clone(), vec![message]).await;
                        }
                        continue;
                    }
                    orchestrate_existing(log_ticks, health_tick, orchestrator, api, connector, container).await
                }
                None => orchestrate_missing(orchestrator, api, connector).await,
            }
        }
        // Iter on each existing container to clean the containers
        let connectors_by_id: HashMap<String, ApiConnector> = connectors
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        log_ticks.retain(|id, _| connectors_by_id.contains_key(id));
        let platform = api.platform();
        let existing_containers = orchestrator.list().await;
        for container in existing_containers {
            let container_platform = container
                .labels
                .get("opencti-platform")
                .map(|value| value.as_str());
            // Only skip containers explicitly belonging to another platform
            if container_platform.is_some() && container_platform != Some(platform) {
                continue;
            }
            let connector_id = container.extract_opencti_id();
            match connectors_by_id.get(&connector_id) {
                None => {
                    // Connector no longer exists — remove the orphaned container
                    orchestrator.remove(&container).await;
                }
                Some(connector) => {
                    // Connector still exists but the deployment name may be stale
                    // after a connector instance name change while the connector ID
                    // remains the same. Remove the old deployment so the next
                    // orchestration cycle deploys with the correct name.
                    let expected_name = connector.container_name();
                    if container.name != expected_name {
                        orchestrator.remove(&container).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiContractConfig;
    use crate::config::settings::Daemon;
    use std::sync::{Arc, Mutex};

    fn connector(id: &str) -> ApiConnector {
        ApiConnector {
            id: id.to_string(),
            platform: "opencti".to_string(),
            name: format!("connector-{id}"),
            image: "ghcr.io/acme/test:latest".to_string(),
            contract_hash: format!("hash-{id}"),
            current_status: Some("stopped".to_string()),
            requested_status: "stopping".to_string(),
            contract_configuration: Vec::<ApiContractConfig>::new(),
        }
    }

    fn managed_container(id: &str, platform: &str) -> OrchestratorContainer {
        let mut labels = HashMap::new();
        labels.insert("opencti-manager".to_string(), "shared-manager".to_string());
        labels.insert("opencti-connector-id".to_string(), id.to_string());
        labels.insert("opencti-platform".to_string(), platform.to_string());

        let mut envs = HashMap::new();
        envs.insert("OPENCTI_CONFIG_HASH".to_string(), format!("hash-{id}"));

        OrchestratorContainer {
            id: format!("container-{id}"),
            name: format!("connector-{}", id.to_lowercase()),
            state: "exited".to_string(),
            labels,
            envs,
            restart_count: 0,
            started_at: None,
        }
    }

    fn legacy_container(id: &str) -> OrchestratorContainer {
        let mut labels = HashMap::new();
        labels.insert("opencti-manager".to_string(), "shared-manager".to_string());
        labels.insert("opencti-connector-id".to_string(), id.to_string());

        let mut envs = HashMap::new();
        envs.insert("OPENCTI_CONFIG_HASH".to_string(), format!("hash-{id}"));

        OrchestratorContainer {
            id: format!("container-{id}"),
            name: format!("connector-{}", id.to_lowercase()),
            state: "exited".to_string(),
            labels,
            envs,
            restart_count: 0,
            started_at: None,
        }
    }

    struct FakeApi {
        connectors: Vec<ApiConnector>,
        pushed_logs: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }

    impl FakeApi {
        fn new(connectors: Vec<ApiConnector>) -> Self {
            Self {
                connectors,
                pushed_logs: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl ComposerApi for FakeApi {
        fn daemon(&self) -> &Daemon {
            unimplemented!()
        }

        fn platform(&self) -> &'static str {
            "opencti"
        }

        fn post_logs_schedule(&self) -> Duration {
            Duration::from_secs(3600)
        }

        async fn version(&self) -> Option<String> {
            unimplemented!()
        }

        async fn ping_alive(&self) -> Option<String> {
            unimplemented!()
        }

        async fn register(&self) -> () {
            unimplemented!()
        }

        async fn connectors(&self) -> Option<Vec<ApiConnector>> {
            Some(self.connectors.clone())
        }

        async fn patch_status(&self, _id: String, _status: ConnectorStatus) -> Option<ApiConnector> {
            None
        }

        async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String> {
            self.pushed_logs
                .lock()
                .expect("mutex should not be poisoned")
                .push((id, logs));
            None
        }

        async fn patch_health(
            &self,
            _id: String,
            _restart_count: u32,
            _started_at: String,
            _is_in_reboot_loop: bool,
        ) -> Option<String> {
            None
        }
    }

    struct FakeOrchestrator {
        containers: Vec<OrchestratorContainer>,
        removed_ids: Arc<Mutex<Vec<String>>>,
        actions: Arc<Mutex<Vec<String>>>,
    }

    impl FakeOrchestrator {
        fn new(containers: Vec<OrchestratorContainer>, removed_ids: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                containers,
                removed_ids,
                actions: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn record(&self, action: &str, connector: &ApiConnector) {
            self.actions
                .lock()
                .expect("mutex should not be poisoned")
                .push(format!("{action}:{}", connector.id));
        }
    }

    #[async_trait::async_trait]
    impl Orchestrator for FakeOrchestrator {
        async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
            // Real orchestrators look containers up by name, which is what makes
            // name collisions possible.
            self.containers
                .iter()
                .find(|container| container.name == connector.container_name())
                .cloned()
        }

        async fn list(&self) -> Vec<OrchestratorContainer> {
            // Real orchestrators only list containers labeled with the manager.
            self.containers
                .iter()
                .filter(|container| container.labels.contains_key("opencti-manager"))
                .cloned()
                .collect()
        }

        async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
            self.record("start", connector);
        }

        async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
            self.record("stop", connector);
        }

        async fn remove(&self, container: &OrchestratorContainer) -> () {
            self.removed_ids
                .lock()
                .expect("mutex should not be poisoned")
                .push(container.extract_opencti_id());
        }

        async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
            self.record("refresh", connector);
            None
        }

        async fn deploy(&self, _connector: &ApiConnector) -> Option<OrchestratorContainer> {
            None
        }

        async fn logs(
            &self,
            _container: &OrchestratorContainer,
            _connector: &ApiConnector,
        ) -> Option<Vec<String>> {
            Some(vec!["connector log line".to_string()])
        }

        fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
            if container.state == "running" {
                ConnectorStatus::Started
            } else {
                ConnectorStatus::Stopped
            }
        }
    }

    #[tokio::test]
    async fn cleanup_does_not_delete_other_platform_connectors_in_shared_mode() {
        let all_containers = vec![
            managed_container("A", "opencti"),
            managed_container("B", "opencti"),
            managed_container("C", "opencti"),
            managed_container("X", "openaev"),
            managed_container("Y", "openaev"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A"), connector("B"), connector("C")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(
            removed.is_empty(),
            "cleanup removed connectors from another platform: {removed:?}"
        );
    }

    #[tokio::test]
    async fn cleanup_removes_only_orphans_for_current_platform() {
        let all_containers = vec![
            managed_container("A", "opencti"),
            managed_container("B", "opencti"),
            managed_container("C", "opencti"),
            managed_container("D", "opencti"),
            managed_container("X", "openaev"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A"), connector("B"), connector("C")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(removed, vec!["D".to_string()]);
    }

    #[tokio::test]
    async fn cleanup_removes_legacy_orphan_without_platform_label() {
        let all_containers = vec![
            managed_container("A", "opencti"),
            legacy_container("Z"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(removed, vec!["Z".to_string()]);
    }

    #[tokio::test]
    async fn cleanup_keeps_legacy_container_with_active_connector() {
        let all_containers = vec![
            managed_container("A", "opencti"),
            legacy_container("B"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A"), connector("B")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(removed.is_empty(), "active legacy container should not be removed: {removed:?}");
    }

    #[tokio::test]
    async fn cleanup_removes_stale_named_container_after_connector_rename() {
        // Simulates OpenAEV 2.4.0 scenario: connector ID stays the same but the
        // name changes (e.g. "connector-A" → "connector-a-0f2a85c1").
        // The old deployment should be removed as orphaned.
        let mut stale_container = managed_container("A", "opencti");
        stale_container.name = "connector-a-old-name".to_string();

        let all_containers = vec![
            stale_container,
            managed_container("B", "opencti"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A"), connector("B")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(
            removed,
            vec!["A".to_string()],
            "stale-named container should be removed"
        );
    }

    #[tokio::test]
    async fn cleanup_keeps_correctly_named_container() {
        // When the container name matches the expected container_name(), it should be kept.
        let all_containers = vec![
            managed_container("A", "opencti"),
            managed_container("B", "opencti"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![connector("A"), connector("B")]));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(removed.is_empty(), "correctly named containers should not be removed: {removed:?}");
    }

    /// Runs one orchestration cycle and returns the (actions, removed ids) seen
    /// by the fake orchestrator.
    async fn run_cycle(
        containers: Vec<OrchestratorContainer>,
        connectors: Vec<ApiConnector>,
    ) -> (Vec<String>, Vec<String>) {
        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator = FakeOrchestrator::new(containers, Arc::clone(&removed_ids));
        let actions = Arc::clone(&orchestrator.actions);
        let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(orchestrator);
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(FakeApi::new(connectors));

        let mut log_ticks = HashMap::new();
        let mut health_tick = Instant::now();

        orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;

        let actions = actions.lock().expect("mutex should not be poisoned").clone();
        let removed = removed_ids.lock().expect("mutex should not be poisoned").clone();
        (actions, removed)
    }

    fn starting_connector(id: &str) -> ApiConnector {
        let mut connector = connector(id);
        connector.requested_status = "starting".to_string();
        connector
    }

    /// Control case for the collision tests below: without a collision, the
    /// connector is reconciled (refreshed on hash change, started/stopped).
    #[tokio::test]
    async fn orchestrate_reconciles_connector_without_collision() {
        let mut outdated = managed_container("A", "opencti");
        outdated
            .envs
            .insert("OPENCTI_CONFIG_HASH".to_string(), "old-hash".to_string());
        let mut running = managed_container("B", "opencti");
        running.state = "running".to_string();

        let (actions, removed) = run_cycle(
            vec![outdated, running],
            vec![starting_connector("A"), connector("B")],
        )
        .await;

        assert_eq!(actions, vec!["refresh:A", "start:A", "stop:B"]);
        assert!(removed.is_empty(), "nothing should be removed: {removed:?}");
    }

    /// Regression test for #162: a same-named deployment not managed by this
    /// manager (e.g. deployed the "old way") must be left completely untouched.
    #[tokio::test]
    async fn orchestrate_skips_connector_on_name_collision_with_foreign_deployment() {
        let mut foreign = managed_container("OTHER", "opencti");
        foreign.name = connector("A").container_name();
        foreign.labels.remove("opencti-manager");

        let (actions, removed) = run_cycle(vec![foreign], vec![starting_connector("A")]).await;

        assert!(actions.is_empty(), "colliding deployment must not be reconciled: {actions:?}");
        assert!(removed.is_empty(), "foreign deployment must not be removed: {removed:?}");
    }

    /// Regression test for #162: a same-named deployment left by a deleted
    /// connector of this manager is not reconciled for the new connector, but
    /// cleaned up as an orphan so the new connector deploys on the next cycle.
    #[tokio::test]
    async fn orchestrate_skips_connector_on_name_collision_and_cleans_orphan() {
        let mut orphan = managed_container("OTHER", "opencti");
        orphan.name = connector("A").container_name();

        let (actions, removed) = run_cycle(vec![orphan], vec![starting_connector("A")]).await;

        assert!(actions.is_empty(), "colliding deployment must not be reconciled: {actions:?}");
        assert_eq!(removed, vec!["OTHER".to_string()]);
    }

    /// Regression test: a same-named deployment without any composer label (e.g.
    /// deployed by hand) used to be reconciled, panicking on the missing config
    /// hash. It must be skipped and left untouched.
    #[tokio::test]
    async fn orchestrate_skips_connector_on_name_collision_with_unlabeled_deployment() {
        let mut unlabeled = managed_container("OTHER", "opencti");
        unlabeled.name = connector("A").container_name();
        unlabeled.labels.clear();
        unlabeled.envs.clear();

        let (actions, removed) = run_cycle(vec![unlabeled], vec![starting_connector("A")]).await;

        assert!(actions.is_empty(), "unlabeled deployment must not be reconciled: {actions:?}");
        assert!(removed.is_empty(), "unlabeled deployment must not be removed: {removed:?}");
    }

    /// Runs `cycles` orchestration cycles sharing the same log schedule state,
    /// starting with the given connectors' logs already due, and returns the
    /// logs pushed to the platform.
    async fn run_cycles_with_due_logs(
        containers: Vec<OrchestratorContainer>,
        connectors: Vec<ApiConnector>,
        cycles: usize,
    ) -> Vec<(String, Vec<String>)> {
        let api = FakeApi::new(connectors.clone());
        let pushed_logs = Arc::clone(&api.pushed_logs);
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(containers, Arc::new(Mutex::new(Vec::new()))));
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(api);

        // FakeApi's logs schedule is 1h: pretend the last push was 2h ago.
        let overdue = Instant::now().checked_sub(Duration::from_secs(7200)).unwrap();
        let mut log_ticks: HashMap<String, Instant> =
            connectors.iter().map(|c| (c.id.clone(), overdue)).collect();
        let mut health_tick = Instant::now();
        for _ in 0..cycles {
            orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;
        }

        let pushed = pushed_logs.lock().expect("mutex should not be poisoned").clone();
        pushed
    }

    /// Regression test: logs used to share a single timer, so only the first
    /// connector of each cycle ever had its logs pushed to the platform.
    #[tokio::test]
    async fn logs_are_pushed_for_every_due_connector_once_per_schedule() {
        let pushed = run_cycles_with_due_logs(
            vec![
                managed_container("A", "opencti"),
                managed_container("B", "opencti"),
                managed_container("C", "opencti"),
            ],
            vec![connector("A"), connector("B"), connector("C")],
            2,
        )
        .await;

        let ids: Vec<&str> = pushed.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B", "C"], "each connector once, not again on the 2nd cycle");
    }

    /// A skipped colliding connector reports why to the platform (on the logs
    /// schedule), not only in the manager logs.
    #[tokio::test]
    async fn collision_is_reported_to_the_platform_once_per_schedule() {
        let mut foreign = managed_container("OTHER", "opencti");
        foreign.name = connector("A").container_name();
        foreign.labels.remove("opencti-manager");

        let pushed = run_cycles_with_due_logs(vec![foreign], vec![connector("A")], 2).await;

        assert_eq!(pushed.len(), 1, "one message per schedule, not every cycle: {pushed:?}");
        let (id, logs) = &pushed[0];
        assert_eq!(id, "A");
        assert!(
            logs[0].contains("Connector not deployed") && logs[0].contains("OTHER"),
            "unexpected message: {logs:?}"
        );
    }

    #[test]
    fn detect_mismatch_returns_found_id_on_collision() {
        let container = managed_container("A", "opencti");
        // Same container (name-based lookup) but reconciled for a different id.
        assert_eq!(
            detect_connector_id_mismatch(&container, "B"),
            Some("A".to_string())
        );
        // No false positive when the ids match.
        assert_eq!(detect_connector_id_mismatch(&container, "A"), None);
        // A container without the label was not deployed by the composer.
        let mut unlabeled = managed_container("A", "opencti");
        unlabeled.labels.clear();
        assert_eq!(
            detect_connector_id_mismatch(&unlabeled, "A"),
            Some("<unlabeled>".to_string())
        );
    }
}
