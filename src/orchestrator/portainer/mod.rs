use std::collections::HashMap;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub mod portainer;

#[derive(Serialize)]
#[serde(rename_all(serialize = "PascalCase"))]
struct PortainerDeployHostConfig {
    network_mode: Option<String>
}

#[derive(Serialize)]
#[serde(rename_all(serialize = "PascalCase"))]
struct PortainerDeployPayload {
    image: String,
    env: Vec<String>,
    labels: HashMap<String, String>,
    host_config: PortainerDeployHostConfig
}

#[derive(Default)]
pub struct PortainerOrchestrator {
    client: Client,
    image_uri: String,
    container_uri: String,
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
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerGetResponseConfig {
    pub labels: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct PortainerGetResponse {
    pub id: String,
    pub image: String,
    pub config: PortainerGetResponseConfig,
    pub state: PortainerGetResponseState,
}
