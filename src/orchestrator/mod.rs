use crate::api::{ApiConnector, ConnectorStatus};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

pub mod composer;
pub mod docker;
pub mod kubernetes;
pub mod portainer;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct OrchestratorContainer {
    pub id: String,
    // pub image: String,
    pub state: String,
    pub labels: HashMap<String, String>,
    pub envs: HashMap<String, String>,
}

impl OrchestratorContainer {
    pub fn is_managed(&self) -> bool {
        self.labels.contains_key("opencti-connector-id")
    }

    pub fn extract_opencti_id(&self) -> String {
        self.labels.get("opencti-connector-id").unwrap().clone()
    }

    pub fn extract_opencti_hash(&self) -> &String {
        self.envs.get("OPENCTI_CONFIG_HASH").unwrap()
    }
}

#[async_trait]
pub trait Orchestrator {
    fn labels(&self, connector: &ApiConnector) -> HashMap<String, String> {
        let settings = crate::settings();
        let mut labels: HashMap<String, String> = HashMap::new();
        labels.insert("opencti-manager".into(), settings.manager.id.clone());
        labels.insert("opencti-connector-id".into(), connector.id.clone());
        labels
    }

    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

    async fn list(&self) -> Option<Vec<OrchestratorContainer>>;

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
