use crate::api::{ApiWorkload, ComposerApi, WorkloadStatus, RequestedStatus};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Detects a workload **name collision**: the container fetched by name for this
/// workload carries a workload-id label that belongs to a *different*
/// workload. This happens when two workloads share the same name (which the platform is
/// meant to forbid) — the orchestrator looks a deployment up by name and returns the
/// wrong one, later surfacing as an obscure Kubernetes
/// "`selector` does not match template `labels`" error.
///
/// Returns the mismatched workload-id found on the container when a collision is
/// present, or `None` when the ids match or the label is absent (legacy/unlabeled
/// containers are not treated as collisions).
fn detect_workload_id_mismatch(
    container: &OrchestratorContainer,
    workload_id: &str,
) -> Option<String> {
    match container.workload_id() {
        Some(found) if found != workload_id => Some(found.clone()),
        _ => None,
    }
}

async fn orchestrate_missing(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    workload: &ApiWorkload,
) {
    // Workload is not provisioned, deploy the images
    let id = workload.id.clone();
    info!(id = id, "Deploying the container");
    let deploy_action = orchestrator.deploy(workload).await;
    match deploy_action {
        // Update the workload status
        Some(_) => {
            api.patch_status(id, WorkloadStatus::Stopped).await;
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
    workload: &ApiWorkload,
    container: OrchestratorContainer,
) {
    // Workload is provisioned
    let workload_id = workload.id.clone();
    let current_status_fetch = workload.current_status.clone().unwrap_or("stopped".into()); // Default current to created
    let workload_status = WorkloadStatus::from_str(current_status_fetch.as_str()).unwrap();
    let requested_status_fetch = workload.requested_status.clone();
    let container_status = orchestrator.state_converter(&container);
    // Check for reboot loop and send health metrics
    let is_in_reboot_loop = container.is_in_reboot_loop();
    let final_status = if is_in_reboot_loop {
        warn!(
            id = workload_id,
            restart_count = container.restart_count,
            "Reboot loop detected"
        );
        // For now, we still report it as Started but with a warning log
        // In the future, we could add a new status like WorkloadStatus::Critical
        container_status
    } else {
        container_status
    };
    
    // Update the workload status if needed
    let container_status_not_aligned = final_status != workload_status;
    
    // Detect if workload just started
    let just_started = container_status_not_aligned && 
                       final_status == WorkloadStatus::Started && 
                       workload_status == WorkloadStatus::Stopped;
    
    // Send health metrics if:
    // - Workload just started (immediate reporting)
    // - OR workload is running and 30 seconds have elapsed
    let now = Instant::now();
    let should_send_health = just_started || 
        (final_status == WorkloadStatus::Started && 
         now.duration_since(health_tick.clone()) >= Duration::from_secs(30));
    
    if should_send_health {
        if let Some(started_at) = &container.started_at {
            info!(id = workload_id, "Reporting health metrics");
            api.patch_health(
                workload_id.clone(),
                container.restart_count,
                started_at.clone(),
                is_in_reboot_loop,
            ).await;
        }
        // Reset timer only for running workloads
        if final_status == WorkloadStatus::Started {
            *health_tick = now;
        }
    }
    if container_status_not_aligned {
        api.patch_status(workload.id.clone(), final_status)
            .await;
        info!(id = workload_id, "Patch status");
    }
    // In case of platform upgrade, we need to align all deployed workloads
    let requested_workload_hash = workload.contract_hash.clone();
    let current_container_hash = container.extract_config_hash();
    if !requested_workload_hash.eq(current_container_hash) {
        // Versions are not aligned
        info!(
            id = workload_id,
            hash = requested_workload_hash,
            "Refreshing"
        );
        orchestrator.refresh(workload).await;
    }
    // Align existing and requested status
    let requested_status = RequestedStatus::from_str(requested_status_fetch.as_str()).unwrap();
    match (requested_status, container_status) {
        (RequestedStatus::Stopping, WorkloadStatus::Started) => {
            info!(id = workload_id, "Stopping");
            orchestrator.stop(&container, workload).await;
        }
        (RequestedStatus::Starting, WorkloadStatus::Stopped) => {
            info!(id = workload_id, "Starting");
            orchestrator.start(&container, workload).await;
        }
        _ => {
            info!(id = workload_id, "Nothing to execute");
        }
    }
    // Get latest logs and update opencti every 5 minutes
    let now = Instant::now();
    if now.duration_since(tick.clone()) >= api.post_logs_schedule() {
        let workload_logs = orchestrator.logs(&container, workload).await;
        match workload_logs {
            Some(logs) => {
                info!(id = workload_id, "Reporting logs");
                api.patch_logs(workload_id, logs).await;
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
    // Get the current definition from the platform
    let workloads_response = api.workloads().await;
    if workloads_response.is_some() {
        // First round trip to instantiate and control if needed
        let workloads = workloads_response.unwrap();
        // Iter on each definition and check alignment between the status and the container
        for workload in &workloads {
            // Get current containers in the orchestrator
            let container_get = orchestrator.get(workload).await;
            match container_get {
                Some(container) => {
                    // Surface workload name collisions clearly: if the container we
                    // got back by name belongs to a different workload-id, two
                    // workloads likely share the same name (OpenCTI should forbid
                    // this). Without this, the only symptom is an obscure Kubernetes
                    // selector-mismatch error.
                    if let Some(found_id) =
                        detect_workload_id_mismatch(&container, &workload.id)
                    {
                        error!(
                            name = workload.name,
                            expected_id = workload.id,
                            found_id = found_id,
                            "Workload name collision detected: an existing deployment with this name belongs to a different workload id (duplicate workload name?)"
                        );
                    }
                    orchestrate_existing(tick, health_tick, orchestrator, api, workload, container).await
                }
                None => orchestrate_missing(orchestrator, api, workload).await,
            }
        }
        // Iter on each existing container to clean the containers
        let workloads_by_id: HashMap<String, ApiWorkload> = workloads
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        let platform = api.platform();
        let existing_containers = orchestrator.list().await;
        for container in existing_containers {
            let container_platform = container.platform().map(|value| value.as_str());
            // Only skip containers explicitly belonging to another platform
            if container_platform.is_some() && container_platform != Some(platform) {
                continue;
            }
            let workload_id = container.extract_workload_id();
            match workloads_by_id.get(&workload_id) {
                None => {
                    // Workload no longer exists — remove the orphaned container
                    orchestrator.remove(&container).await;
                }
                Some(workload) => {
                    // Workload still exists but the deployment name may be stale
                    // after a workload instance name change while the workload ID
                    // remains the same. Remove the old deployment so the next
                    // orchestration cycle deploys with the correct name.
                    let expected_name = workload.container_name();
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
    use crate::orchestrator::{ENV_CONFIG_HASH, LABEL_MANAGER, LABEL_PLATFORM, LABEL_WORKLOAD_ID};
    use std::sync::{Arc, Mutex};

    fn workload(id: &str) -> ApiWorkload {
        ApiWorkload {
            id: id.to_string(),
            platform: "opencti".to_string(),
            name: format!("workload-{id}"),
            image: "ghcr.io/acme/test:latest".to_string(),
            contract_hash: format!("hash-{id}"),
            current_status: Some("stopped".to_string()),
            requested_status: "stopping".to_string(),
            contract_configuration: Vec::<ApiContractConfig>::new(),
        }
    }

    fn managed_container(id: &str, platform: &str) -> OrchestratorContainer {
        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGER.to_string(), "shared-manager".to_string());
        labels.insert(LABEL_WORKLOAD_ID.to_string(), id.to_string());
        labels.insert(LABEL_PLATFORM.to_string(), platform.to_string());

        let mut envs = HashMap::new();
        envs.insert(ENV_CONFIG_HASH.to_string(), format!("hash-{id}"));

        OrchestratorContainer {
            id: format!("container-{id}"),
            name: format!("workload-{}", id.to_lowercase()),
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
            name: format!("workload-{}", id.to_lowercase()),
            state: "exited".to_string(),
            labels,
            envs,
            restart_count: 0,
            started_at: None,
        }
    }

    struct FakeApi {
        workloads: Vec<ApiWorkload>,
    }

    impl FakeApi {
        fn new(workloads: Vec<ApiWorkload>) -> Self {
            Self { workloads }
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

        async fn workloads(&self) -> Option<Vec<ApiWorkload>> {
            Some(self.workloads.clone())
        }

        async fn patch_status(&self, _id: String, _status: WorkloadStatus) -> Option<ApiWorkload> {
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
        async fn get(&self, workload: &ApiWorkload) -> Option<OrchestratorContainer> {
            self.containers
                .iter()
                .find(|container| container.workload_id() == Some(&workload.id))
                .cloned()
        }

        async fn list(&self) -> Vec<OrchestratorContainer> {
            self.containers.clone()
        }

        async fn start(&self, _container: &OrchestratorContainer, _workload: &ApiWorkload) -> () {}

        async fn stop(&self, _container: &OrchestratorContainer, _workload: &ApiWorkload) -> () {}

        async fn remove(&self, container: &OrchestratorContainer) -> () {
            self.removed_ids
                .lock()
                .expect("mutex should not be poisoned")
                .push(container.extract_workload_id());
        }

        async fn refresh(&self, _workload: &ApiWorkload) -> Option<OrchestratorContainer> {
            None
        }

        async fn deploy(&self, _workload: &ApiWorkload) -> Option<OrchestratorContainer> {
            None
        }

        async fn logs(
            &self,
            _container: &OrchestratorContainer,
            _workload: &ApiWorkload,
        ) -> Option<Vec<String>> {
            None
        }

        fn state_converter(&self, container: &OrchestratorContainer) -> WorkloadStatus {
            if container.state == "running" {
                WorkloadStatus::Started
            } else {
                WorkloadStatus::Stopped
            }
        }
    }

    #[tokio::test]
    async fn cleanup_does_not_delete_other_platform_workloads_in_shared_mode() {
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
            Box::new(FakeApi::new(vec![workload("A"), workload("B"), workload("C")]));

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(
            removed.is_empty(),
            "cleanup removed workloads from another platform: {removed:?}"
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
            Box::new(FakeApi::new(vec![workload("A"), workload("B"), workload("C")]));

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
            Box::new(FakeApi::new(vec![workload("A")]));

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
    async fn cleanup_keeps_legacy_container_with_active_workload() {
        let all_containers = vec![
            managed_container("A", "opencti"),
            legacy_container("B"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![workload("A"), workload("B")]));

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
    async fn cleanup_removes_stale_named_container_after_workload_rename() {
        // Simulates OpenAEV 2.4.0 scenario: workload ID stays the same but the
        // name changes (e.g. "workload-A" → "workload-a-0f2a85c1").
        // The old deployment should be removed as orphaned.
        let mut stale_container = managed_container("A", "opencti");
        stale_container.name = "workload-a-old-name".to_string();

        let all_containers = vec![
            stale_container,
            managed_container("B", "opencti"),
        ];

        let removed_ids = Arc::new(Mutex::new(Vec::new()));
        let orchestrator: Box<dyn Orchestrator + Send + Sync> =
            Box::new(FakeOrchestrator::new(all_containers, Arc::clone(&removed_ids)));
        let api: Box<dyn ComposerApi + Send + Sync> =
            Box::new(FakeApi::new(vec![workload("A"), workload("B")]));

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
            Box::new(FakeApi::new(vec![workload("A"), workload("B")]));

        let mut tick = Instant::now();
        let mut health_tick = Instant::now();

        orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

        let removed = removed_ids
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert!(removed.is_empty(), "correctly named containers should not be removed: {removed:?}");
    }

    #[test]
    fn detect_mismatch_returns_found_id_on_collision() {
        let container = managed_container("A", "opencti");
        // Same container (name-based lookup) but reconciled for a different id.
        assert_eq!(
            detect_workload_id_mismatch(&container, "B"),
            Some("A".to_string())
        );
        // No false positive when the ids match.
        assert_eq!(detect_workload_id_mismatch(&container, "A"), None);
    }
}
