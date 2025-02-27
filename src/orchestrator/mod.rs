use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Deserialize};
use crate::api::connector::{Connector, ConnectorCurrentStatus};
use crate::config::settings::Settings;

pub mod kubernetes;
pub mod docker;
pub mod portainer;
pub mod composer;

#[derive(Deserialize, Clone)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct OrchestratorContainer {
    pub id: String,
    pub image: String,
    pub state: String,
    pub labels: HashMap<String, String>,
}

impl OrchestratorContainer  {
    pub fn is_managed(&self) -> bool {
        self.labels.contains_key("opencti-connector-id")
    }
    pub fn extract_opencti_id(&self) -> &String {
        self.labels.get("opencti-connector-id").unwrap()
    }
}

#[async_trait]
pub trait Orchestrator {
    async fn container(&self, container_id: String) -> Option<OrchestratorContainer>;

    async fn containers(&self) -> Option<Vec<OrchestratorContainer>>;

    async fn container_start(&self, container_id: String) -> ();

    async fn container_stop(&self, container_id: String) -> ();

    async fn container_deploy(&self, settings_data: &Settings, connector: &Connector) -> Option<OrchestratorContainer>;

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus;
}