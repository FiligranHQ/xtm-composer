//! Test-only in-memory orchestrator shared by orchestrator and engine tests.
use crate::api::{ApiConnector, ConnectorStatus};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::sync::{Arc, Mutex};

pub struct FakeOrchestrator {
    containers: Vec<OrchestratorContainer>,
    removed_ids: Arc<Mutex<Vec<String>>>,
    pub actions: Arc<Mutex<Vec<String>>>,
}

impl FakeOrchestrator {
    pub fn new(
        containers: Vec<OrchestratorContainer>,
        removed_ids: Arc<Mutex<Vec<String>>>,
    ) -> Self {
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
