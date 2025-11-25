use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::orchestrator::composer::{orchestrate};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

// Mock Orchestrator for testing
struct MockOrchestrator {
    containers: Arc<Mutex<HashMap<String, OrchestratorContainer>>>,
    deploy_should_fail: bool,
    start_should_fail: bool,
    stop_should_fail: bool,
}

impl MockOrchestrator {
    fn new() -> Self {
        Self {
            containers: Arc::new(Mutex::new(HashMap::new())),
            deploy_should_fail: false,
            start_should_fail: false,
            stop_should_fail: false,
        }
    }

    fn with_failure_modes(deploy_fail: bool, start_fail: bool, stop_fail: bool) -> Self {
        Self {
            containers: Arc::new(Mutex::new(HashMap::new())),
            deploy_should_fail: deploy_fail,
            start_should_fail: start_fail,
            stop_should_fail: stop_fail,
        }
    }
}

#[async_trait]
impl Orchestrator for MockOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let containers = self.containers.lock().await;
        containers.get(&connector.id).cloned()
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let containers = self.containers.lock().await;
        containers.values().cloned().collect()
    }

    async fn start(&self, container: &OrchestratorContainer, _connector: &ApiConnector) {
        if !self.start_should_fail {
            let mut containers = self.containers.lock().await;
            let connector_id = container.extract_opencti_id();
            if let Some(c) = containers.get_mut(&connector_id) {
                c.state = "running".to_string();
            }
        }
    }

    async fn stop(&self, container: &OrchestratorContainer, _connector: &ApiConnector) {
        if !self.stop_should_fail {
            let mut containers = self.containers.lock().await;
            let connector_id = container.extract_opencti_id();
            if let Some(c) = containers.get_mut(&connector_id) {
                c.state = "exited".to_string();
            }
        }
    }

    async fn remove(&self, container: &OrchestratorContainer) {
        let mut containers = self.containers.lock().await;
        let connector_id = container.extract_opencti_id();
        containers.remove(&connector_id);
    }

    async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let mut containers = self.containers.lock().await;
        if let Some(container) = containers.get_mut(&connector.id) {
            // Update hash to match connector
            container.envs.insert(
                "OPENCTI_CONFIG_HASH".to_string(),
                connector.contract_hash.clone(),
            );
            Some(container.clone())
        } else {
            None
        }
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        if self.deploy_should_fail {
            return None;
        }

        let mut labels = HashMap::new();
        labels.insert("opencti-connector-id".to_string(), connector.id.clone());

        let mut envs = HashMap::new();
        envs.insert(
            "OPENCTI_CONFIG_HASH".to_string(),
            connector.contract_hash.clone(),
        );

        let container = OrchestratorContainer {
            id: format!("container-{}", connector.id),
            name: connector.container_name(),
            state: "exited".to_string(),
            labels,
            envs,
            restart_count: 0,
            started_at: None,
        };

        let mut containers = self.containers.lock().await;
        containers.insert(connector.id.clone(), container.clone());
        Some(container)
    }

    async fn logs(
        &self,
        _container: &OrchestratorContainer,
        _connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        Some(vec!["test log line 1".to_string(), "test log line 2".to_string()])
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" => ConnectorStatus::Started,
            _ => ConnectorStatus::Stopped,
        }
    }
}

// Mock API for testing
struct MockComposerApi {
    connectors: Arc<Mutex<Vec<ApiConnector>>>,
    status_updates: Arc<Mutex<Vec<(String, ConnectorStatus)>>>,
    health_updates: Arc<Mutex<Vec<String>>>,
    log_updates: Arc<Mutex<Vec<String>>>,
}

impl MockComposerApi {
    fn new(connectors: Vec<ApiConnector>) -> Self {
        Self {
            connectors: Arc::new(Mutex::new(connectors)),
            status_updates: Arc::new(Mutex::new(Vec::new())),
            health_updates: Arc::new(Mutex::new(Vec::new())),
            log_updates: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ComposerApi for MockComposerApi {
    fn daemon(&self) -> &crate::config::settings::Daemon {
        unimplemented!("Not needed for these tests")
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes
    }

    async fn version(&self) -> Option<String> {
        Some("1.0.0".to_string())
    }

    async fn ping_alive(&self) -> Option<String> {
        Some("pong".to_string())
    }

    async fn register(&self) {}

    async fn connectors(&self) -> Option<Vec<ApiConnector>> {
        let connectors = self.connectors.lock().await;
        Some(connectors.clone())
    }

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector> {
        let mut updates = self.status_updates.lock().await;
        updates.push((id.clone(), status));

        let mut connectors = self.connectors.lock().await;
        if let Some(connector) = connectors.iter_mut().find(|c| c.id == id) {
            connector.current_status = Some(match status {
                ConnectorStatus::Started => "started".to_string(),
                ConnectorStatus::Stopped => "stopped".to_string(),
            });
            Some(connector.clone())
        } else {
            None
        }
    }

    async fn patch_logs(&self, id: String, _logs: Vec<String>) -> Option<cynic::Id> {
        let mut updates = self.log_updates.lock().await;
        updates.push(id);
        Some(cynic::Id::new("log-id"))
    }

    async fn patch_health(
        &self,
        id: String,
        _restart_count: u32,
        _started_at: String,
        _is_in_reboot_loop: bool,
    ) -> Option<cynic::Id> {
        let mut updates = self.health_updates.lock().await;
        updates.push(id);
        Some(cynic::Id::new("health-id"))
    }
}

#[tokio::test]
async fn test_orchestrate_missing_connector_deploy_success() {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-1", "nginx:latest");
    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify container was deployed
    let deployed = orchestrator.get(&connector).await;
    assert!(deployed.is_some());

    let container = deployed.unwrap();
    assert_eq!(container.name, connector.container_name());
}

#[tokio::test]
async fn test_orchestrate_missing_connector_deploy_failure() {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-2", "nonexistent:latest");
    let orchestrator: Box<dyn Orchestrator + Send + Sync> =
        Box::new(MockOrchestrator::with_failure_modes(true, false, false));
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify container was not deployed
    let deployed = orchestrator.get(&connector).await;
    assert!(deployed.is_none());
}

#[tokio::test]
async fn test_orchestrate_existing_connector_start() {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-3", "nginx:latest");
    connector.requested_status = "starting".to_string();
    connector.current_status = Some("stopped".to_string());

    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    
    // Pre-deploy the connector
    let _ = orchestrator.deploy(&connector).await;

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify container was started
    let container = orchestrator.get(&connector).await.unwrap();
    assert_eq!(container.state, "running");
}

#[tokio::test]
async fn test_orchestrate_existing_connector_stop() {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-4", "nginx:latest");
    connector.requested_status = "stopping".to_string();
    connector.current_status = Some("started".to_string());

    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    
    // Pre-deploy and start the connector
    let container = orchestrator.deploy(&connector).await.unwrap();
    orchestrator.start(&container, &connector).await;

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify container was stopped
    let container = orchestrator.get(&connector).await.unwrap();
    assert_eq!(container.state, "exited");
}

#[tokio::test]
async fn test_orchestrate_hash_mismatch_triggers_refresh() {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-5", "nginx:latest");
    connector.contract_hash = "new-hash-xyz".to_string();
    connector.current_status = Some("started".to_string());

    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    
    // Pre-deploy with old hash
    orchestrator.deploy(&connector).await;
    
    // Change hash
    connector.contract_hash = "updated-hash-abc".to_string();

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify hash was updated
    let container = orchestrator.get(&connector).await.unwrap();
    assert_eq!(
        container.envs.get("OPENCTI_CONFIG_HASH"),
        Some(&connector.contract_hash)
    );
}

#[tokio::test]
async fn test_orchestrate_removes_orphaned_containers() {
    use super::test_helpers::create_test_connector;

    let connector1 = create_test_connector("test-6", "nginx:latest");
    let connector2 = create_test_connector("test-7", "redis:latest");

    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    
    // Deploy both connectors
    orchestrator.deploy(&connector1).await;
    orchestrator.deploy(&connector2).await;

    // Only include connector1 in API response
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector1.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify connector1 still exists
    assert!(orchestrator.get(&connector1).await.is_some());

    // Verify connector2 was removed (orphaned)
    assert!(orchestrator.get(&connector2).await.is_none());
}

#[tokio::test]
async fn test_orchestrate_multiple_connectors_parallel() {
    use super::test_helpers::create_test_connector;

    let connector1 = create_test_connector("test-8", "nginx:latest");
    let connector2 = create_test_connector("test-9", "redis:latest");
    let connector3 = create_test_connector("test-10", "postgres:latest");

    let orchestrator: Box<dyn Orchestrator + Send + Sync> = Box::new(MockOrchestrator::new());
    let api: Box<dyn ComposerApi + Send + Sync> = Box::new(MockComposerApi::new(vec![
        connector1.clone(),
        connector2.clone(),
        connector3.clone(),
    ]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;

    // Verify all containers were deployed
    assert!(orchestrator.get(&connector1).await.is_some());
    assert!(orchestrator.get(&connector2).await.is_some());
    assert!(orchestrator.get(&connector3).await.is_some());
}

#[tokio::test]
async fn test_orchestrate_reboot_loop_detection() {
    use super::test_helpers::create_test_container_with_restarts;
    use chrono::Utc;

    let container = create_test_container_with_restarts(
        "test-11",
        "test-connector",
        5, // High restart count
        Some(Utc::now().to_rfc3339()), // Just started
    );

    // Test reboot loop detection
    assert!(container.is_in_reboot_loop());

    // Test with old start time (no reboot loop)
    let old_container = create_test_container_with_restarts(
        "test-12",
        "test-connector",
        5,
        Some("2024-01-01T00:00:00Z".to_string()),
    );

    assert!(!old_container.is_in_reboot_loop());
}

#[test]
fn test_container_is_managed() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-13", "test-container", "running");
    assert!(container.is_managed());

    // Container without opencti-connector-id label
    let mut unmanaged = container.clone();
    unmanaged.labels.remove("opencti-connector-id");
    assert!(!unmanaged.is_managed());
}

#[test]
fn test_container_extract_opencti_id() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-14", "test-container", "running");
    assert_eq!(container.extract_opencti_id(), "test-14");
}

#[test]
fn test_container_extract_opencti_hash() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-15", "test-container", "running");
    assert_eq!(container.extract_opencti_hash(), "test-hash-abc123");
}
