use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::orchestrator::composer::{orchestrate};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use rstest::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

// Fixtures for test setup
#[fixture]
fn mock_orchestrator() -> Box<dyn Orchestrator + Send + Sync> {
    Box::new(MockOrchestrator::new())
}

#[fixture]
fn failing_orchestrator() -> Box<dyn Orchestrator + Send + Sync> {
    Box::new(MockOrchestrator::with_failure_modes(true, false, false))
}

#[fixture]
fn single_connector_api() -> Box<dyn ComposerApi + Send + Sync> {
    use super::test_helpers::create_test_connector;
    let connector = create_test_connector("test-default", "nginx:latest");
    Box::new(MockComposerApi::new(vec![connector]))
}

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

#[rstest]
#[tokio::test]
async fn test_orchestrate_missing_connector_deploy_success(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-1", "nginx:latest");
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify container was deployed
    let deployed = mock_orchestrator.get(&connector).await;
    assert!(deployed.is_some());

    let container = deployed.unwrap();
    assert_eq!(container.name, connector.container_name());
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_missing_connector_deploy_failure(
    failing_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-2", "nonexistent:latest");
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &failing_orchestrator, &api).await;

    // Verify container was not deployed
    let deployed = failing_orchestrator.get(&connector).await;
    assert!(deployed.is_none());
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_existing_connector_start(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-3", "nginx:latest");
    connector.requested_status = "starting".to_string();
    connector.current_status = Some("stopped".to_string());
    
    // Pre-deploy the connector
    let _ = mock_orchestrator.deploy(&connector).await;

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify container was started
    let container = mock_orchestrator.get(&connector).await.unwrap();
    assert_eq!(container.state, "running");
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_existing_connector_stop(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-4", "nginx:latest");
    connector.requested_status = "stopping".to_string();
    connector.current_status = Some("started".to_string());
    
    // Pre-deploy and start the connector
    let container = mock_orchestrator.deploy(&connector).await.unwrap();
    mock_orchestrator.start(&container, &connector).await;

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify container was stopped
    let container = mock_orchestrator.get(&connector).await.unwrap();
    assert_eq!(container.state, "exited");
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_hash_mismatch_triggers_refresh(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let mut connector = create_test_connector("test-5", "nginx:latest");
    connector.contract_hash = "new-hash-xyz".to_string();
    connector.current_status = Some("started".to_string());
    
    // Pre-deploy with old hash
    mock_orchestrator.deploy(&connector).await;
    
    // Change hash
    connector.contract_hash = "updated-hash-abc".to_string();

    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify hash was updated
    let container = mock_orchestrator.get(&connector).await.unwrap();
    assert_eq!(
        container.envs.get("OPENCTI_CONFIG_HASH"),
        Some(&connector.contract_hash)
    );
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_removes_orphaned_containers(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let connector1 = create_test_connector("test-6", "nginx:latest");
    let connector2 = create_test_connector("test-7", "redis:latest");
    
    // Deploy both connectors
    mock_orchestrator.deploy(&connector1).await;
    mock_orchestrator.deploy(&connector2).await;

    // Only include connector1 in API response
    let api: Box<dyn ComposerApi + Send + Sync> =
        Box::new(MockComposerApi::new(vec![connector1.clone()]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify connector1 still exists
    assert!(mock_orchestrator.get(&connector1).await.is_some());

    // Verify connector2 was removed (orphaned)
    assert!(mock_orchestrator.get(&connector2).await.is_none());
}

#[rstest]
#[tokio::test]
async fn test_orchestrate_multiple_connectors_parallel(
    mock_orchestrator: Box<dyn Orchestrator + Send + Sync>,
) {
    use super::test_helpers::create_test_connector;

    let connector1 = create_test_connector("test-8", "nginx:latest");
    let connector2 = create_test_connector("test-9", "redis:latest");
    let connector3 = create_test_connector("test-10", "postgres:latest");

    let api: Box<dyn ComposerApi + Send + Sync> = Box::new(MockComposerApi::new(vec![
        connector1.clone(),
        connector2.clone(),
        connector3.clone(),
    ]));

    let mut tick = std::time::Instant::now();
    let mut health_tick = std::time::Instant::now();

    orchestrate(&mut tick, &mut health_tick, &mock_orchestrator, &api).await;

    // Verify all containers were deployed
    assert!(mock_orchestrator.get(&connector1).await.is_some());
    assert!(mock_orchestrator.get(&connector2).await.is_some());
    assert!(mock_orchestrator.get(&connector3).await.is_some());
}

#[rstest]
#[case(5, true)]  // High restart count + recent start = reboot loop
#[case(5, false)] // High restart count + old start = no reboot loop
#[tokio::test]
async fn test_orchestrate_reboot_loop_detection(
    #[case] restart_count: u32,
    #[case] is_recent: bool,
) {
    use super::test_helpers::create_test_container_with_restarts;
    use chrono::Utc;

    let started_at = if is_recent {
        Some(Utc::now().to_rfc3339())
    } else {
        Some("2024-01-01T00:00:00Z".to_string())
    };

    let container = create_test_container_with_restarts(
        "test-11",
        "test-connector",
        restart_count,
        started_at,
    );

    assert_eq!(container.is_in_reboot_loop(), is_recent);
}

#[rstest]
#[case(true, true)]   // With label = managed
#[case(false, false)] // Without label = not managed
#[test]
fn test_container_is_managed(
    #[case] has_label: bool,
    #[case] expected_managed: bool,
) {
    use super::test_helpers::create_test_container;

    let mut container = create_test_container("test-13", "test-container", "running");
    
    if !has_label {
        container.labels.remove("opencti-connector-id");
    }
    
    assert_eq!(container.is_managed(), expected_managed);
}

#[rstest]
#[case("test-14", "test-14")]
#[case("connector-123", "connector-123")]
#[test]
fn test_container_extract_opencti_id(
    #[case] id: &str,
    #[case] expected: &str,
) {
    use super::test_helpers::create_test_container;

    let container = create_test_container(id, "test-container", "running");
    assert_eq!(container.extract_opencti_id(), expected);
}

#[rstest]
#[test]
fn test_container_extract_opencti_hash() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-15", "test-container", "running");
    assert_eq!(container.extract_opencti_hash(), "test-hash-abc123");
}
