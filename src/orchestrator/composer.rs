use crate::api::{ApiConnector, ComposerApi, ConnectorStatus, RequestedStatus};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{info, warn};

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
    tick: &mut Instant,
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
    // Get latest logs and update opencti every 5 minutes
    let now = Instant::now();
    if now.duration_since(tick.clone()) >= api.post_logs_schedule() {
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
        *tick = now;
    }
}

pub async fn orchestrate(
    tick: &mut Instant,
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
                    orchestrate_existing(tick, health_tick, orchestrator, api, connector, container).await
                }
                None => orchestrate_missing(orchestrator, api, connector).await,
            }
        }
        // Iter on each existing container to clean the containers
        let connectors_by_id: HashMap<String, ApiConnector> = connectors
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
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
                    // (e.g. after a platform rename). Remove the old deployment so
                    // the next orchestration cycle deploys with the correct name.
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
    }

    impl FakeApi {
        fn new(connectors: Vec<ApiConnector>) -> Self {
            Self { connectors }
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

        async fn patch_logs(&self, _id: String, _logs: Vec<String>) -> Option<String> {
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
    }

    impl FakeOrchestrator {
        fn new(containers: Vec<OrchestratorContainer>, removed_ids: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                containers,
                removed_ids,
            }
        }
    }

    #[async_trait::async_trait]
    impl Orchestrator for FakeOrchestrator {
        async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
            self.containers
                .iter()
                .find(|container| container.labels.get("opencti-connector-id") == Some(&connector.id))
                .cloned()
        }

        async fn list(&self) -> Vec<OrchestratorContainer> {
            self.containers.clone()
        }

        async fn start(&self, _container: &OrchestratorContainer, _connector: &ApiConnector) -> () {}

        async fn stop(&self, _container: &OrchestratorContainer, _connector: &ApiConnector) -> () {}

        async fn remove(&self, container: &OrchestratorContainer) -> () {
            self.removed_ids
                .lock()
                .expect("mutex should not be poisoned")
                .push(container.extract_opencti_id());
        }

        async fn refresh(&self, _connector: &ApiConnector) -> Option<OrchestratorContainer> {
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
            None
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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

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

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(removed.is_empty(), "correctly named containers should not be removed: {removed:?}");
    }
}
