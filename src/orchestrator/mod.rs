use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Deserialize};

pub mod portainer;
pub mod kube;
pub mod docker;

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct OrchestratorContainer {
    pub id: String,
    pub image: String,
    pub labels: HashMap<String, String>,
}

impl OrchestratorContainer  {
    pub fn is_managed(&self) -> bool {
        self.labels.contains_key("opencti-connector-id")
    }
}

#[async_trait]
pub trait Orchestrator {
    async fn containers(&self) -> Option<Vec<OrchestratorContainer>>;
    // async fn start(&self) -> bool;
    // async fn stop(&self) -> bool;
    // async fn kill(&self) -> bool;
}