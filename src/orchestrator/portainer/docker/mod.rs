use crate::config::settings::Portainer;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod portainer;

#[derive(Serialize)]
#[serde(rename_all(serialize = "PascalCase"))]
pub struct PortainerDeployHostConfig {
    pub network_mode: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all(serialize = "PascalCase"))]
pub struct PortainerDeployPayload {
    pub image: String,
    pub env: Vec<String>,
    pub labels: HashMap<String, String>,
    pub host_config: PortainerDeployHostConfig,
}

pub struct PortainerDockerOrchestrator {
    pub client: Client,
    pub image_uri: String,
    pub container_uri: String,
    pub config: Portainer,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerDeployResponse {
    pub id: String,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerGetResponseState {
    pub status: String,
    pub started_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerGetResponseConfig {
    pub env: Vec<String>,
    pub labels: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerGetResponse {
    pub id: String,
    pub name: String,
    pub config: PortainerGetResponseConfig,
    pub state: PortainerGetResponseState,
    pub restart_count: Option<i64>,
}
