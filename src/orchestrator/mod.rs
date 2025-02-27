use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Deserialize};
use crate::api::connector::{Connector};

pub mod portainer;
pub mod kube;
pub mod docker;

// enum OrchestratorContainerStatus {
//     Created  // created
//     Stopped, // stopped / exited
//     Running, // running / healthy
// }

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

    async fn container_start(&self, connector_id: String) -> ();

    async fn container_deploy(&self, connector: &Connector) -> Option<OrchestratorContainer>;
    // async fn start(&self) -> bool;
    // async fn stop(&self) -> bool;
    // async fn kill(&self) -> bool;
}