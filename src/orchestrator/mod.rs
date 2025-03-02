use crate::api::connector::{ConnectorCurrentStatus, ManagedConnector};
use crate::config::settings::Settings;
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
    pub image: String,
    pub state: String,
    pub labels: HashMap<String, String>,
}

impl OrchestratorContainer {
    pub fn is_managed(&self) -> bool {
        self.labels.contains_key("opencti-connector-id")
    }
    pub fn extract_opencti_id(&self) -> &String {
        self.labels.get("opencti-connector-id").unwrap()
    }
}

#[async_trait]
pub trait Orchestrator {
    async fn container(
        &self,
        container_id: String,
        connector: &ManagedConnector,
    ) -> Option<OrchestratorContainer>;

    async fn list(&self, connector: &ManagedConnector) -> Option<Vec<OrchestratorContainer>>;

    async fn start(&self, container: &OrchestratorContainer, connector: &ManagedConnector) -> ();

    async fn stop(&self, container: &OrchestratorContainer, connector: &ManagedConnector) -> ();

    async fn deploy(
        &self,
        settings: &Settings,
        connector: &ManagedConnector,
    ) -> Option<OrchestratorContainer>;

    async fn logs(
        &self,
        container: &OrchestratorContainer,
        connector: &ManagedConnector,
    ) -> Option<Vec<String>>;

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus;
}
