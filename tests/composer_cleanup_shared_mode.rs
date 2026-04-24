use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod api {
    use async_trait::async_trait;
    use std::str::FromStr;
    use std::time::Duration;

    #[allow(dead_code)]
    #[derive(Debug, Clone)]
    pub struct ApiContractConfig {
        pub key: String,
        pub value: String,
        pub is_sensitive: bool,
    }

    #[allow(dead_code)]
    #[derive(Debug, Clone)]
    pub struct ApiConnector {
        pub id: String,
        pub platform: String,
        pub name: String,
        pub image: String,
        pub contract_hash: String,
        pub current_status: Option<String>,
        pub requested_status: String,
        pub contract_configuration: Vec<ApiContractConfig>,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum ConnectorStatus {
        Started,
        Stopped,
    }

    impl FromStr for ConnectorStatus {
        type Err = ();

        fn from_str(input: &str) -> Result<ConnectorStatus, Self::Err> {
            match input {
                "created" | "exited" => Ok(ConnectorStatus::Stopped),
                "started" | "healthy" | "running" => Ok(ConnectorStatus::Started),
                _ => Ok(ConnectorStatus::Stopped),
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum RequestedStatus {
        Starting,
        Stopping,
    }

    impl FromStr for RequestedStatus {
        type Err = ();

        fn from_str(input: &str) -> Result<RequestedStatus, Self::Err> {
            match input {
                "starting" => Ok(RequestedStatus::Starting),
                "stopping" => Ok(RequestedStatus::Stopping),
                _ => Ok(RequestedStatus::Stopping),
            }
        }
    }

    #[async_trait]
    pub trait ComposerApi {
        fn platform(&self) -> &'static str;

        fn post_logs_schedule(&self) -> Duration;

        async fn connectors(&self) -> Option<Vec<ApiConnector>>;

        async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector>;

        async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String>;

        async fn patch_health(
            &self,
            id: String,
            restart_count: u32,
            started_at: String,
            is_in_reboot_loop: bool,
        ) -> Option<String>;
    }
}

mod orchestrator {
    use crate::api::{ApiConnector, ConnectorStatus};
    use async_trait::async_trait;
    use chrono::{DateTime, Duration, Utc};
    use std::collections::HashMap;

    #[allow(dead_code)]
    #[derive(Clone, Debug)]
    pub struct OrchestratorContainer {
        pub id: String,
        pub name: String,
        pub state: String,
        pub labels: HashMap<String, String>,
        pub envs: HashMap<String, String>,
        pub restart_count: u32,
        pub started_at: Option<String>,
    }

    impl OrchestratorContainer {
        pub fn extract_opencti_id(&self) -> String {
            self.labels
                .get("opencti-connector-id")
                .expect("missing opencti-connector-id")
                .clone()
        }

        pub fn extract_opencti_hash(&self) -> &String {
            self.envs
                .get("OPENCTI_CONFIG_HASH")
                .expect("missing OPENCTI_CONFIG_HASH")
        }

        pub fn is_in_reboot_loop(&self) -> bool {
            if self.restart_count > 3 {
                if let Some(started_at_str) = &self.started_at {
                    if let Ok(started_at) = DateTime::parse_from_rfc3339(started_at_str) {
                        let uptime = Utc::now() - started_at.with_timezone(&Utc);
                        return uptime < Duration::minutes(5);
                    }
                }
            }
            false
        }
    }

    #[async_trait]
    pub trait Orchestrator {
        async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

        async fn list(&self) -> Vec<OrchestratorContainer>;

        async fn start(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> ();

        async fn stop(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> ();

        async fn remove(&self, container: &OrchestratorContainer) -> ();

        async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

        async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

        async fn logs(
            &self,
            container: &OrchestratorContainer,
            connector: &ApiConnector,
        ) -> Option<Vec<String>>;

        fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus;
    }

    pub mod composer {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/orchestrator/composer.rs"));
    }
}

use api::{ApiConnector, ApiContractConfig, ComposerApi, ConnectorStatus};
use orchestrator::composer::orchestrate;
use orchestrator::{Orchestrator, OrchestratorContainer};

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
        name: format!("connector-{id}"),
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

#[async_trait]
impl ComposerApi for FakeApi {
    fn platform(&self) -> &'static str {
        "opencti"
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(3600)
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

#[async_trait]
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





