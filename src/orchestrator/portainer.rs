use std::collections::HashMap;
use async_trait::async_trait;
use header::HeaderValue;
use log::{error, info};
use reqwest::{header, Client};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use crate::api::connector::{Connector};
use crate::config::settings::Portainer;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};

const X_API_KEY: &str = "X-API-KEY";

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


impl PortainerOrchestrator {
    pub fn new(config: &Portainer) -> Self {
        let container_uri = format!("{}/api/endpoints/{}/docker/{}/containers",
                               config.api, config.env_id, config.api_version);
        let image_uri = format!("{}/api/endpoints/{}/docker/{}/images",
                               config.api, config.env_id, config.api_version);
        let mut headers = HeaderMap::new();
        headers.insert(X_API_KEY, HeaderValue::from_bytes(config.api_key.as_bytes()).unwrap());
        let client = Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build().unwrap();
        Self { image_uri, container_uri, client }
    }
}

// GET https://localhost:9443/api/endpoints/3/docker/v1.41/containers/json?all=true
// POST https://localhost:9443/api/endpoints/3/docker/v1.41/containers/create?name=test
// POST https://localhost:9443/api/endpoints/3/docker/v1.41/containers/803eb3a4fa131d2823c0d9ae78368d51326445744e55d6441446b4ccb6b415d1/start
// DEL https://localhost:9443/api/endpoints/3/docker/v1.41/containers/803eb3a4fa131d2823c0d9ae78368d51326445744e55d6441446b4ccb6b415d1?v=0&force=true
#[async_trait]
impl Orchestrator for PortainerOrchestrator {

    async fn container(&self, container_id: String) -> Option<OrchestratorContainer> {
        let container_uri = format!("{}/{}/json", self.container_uri, container_id);
        let response = self.client.get(container_uri).send().await;
        let response_data: PortainerGetResponse = response.unwrap().json().await.unwrap();
        Some(OrchestratorContainer {
            id: response_data.id,
            state: response_data.state.status,
            labels: response_data.config.labels,
            image: response_data.image
        })
    }

    async fn containers(&self) -> Option<Vec<OrchestratorContainer>> {
        let list_uri = format!("{}/json?all=true", self.container_uri);
        let response = self.client.get(list_uri).send().await;
        let response_result = match response {
            Ok(data) => data.json().await,
            Err(err) => {
                error!("Portainer error fetching containers: {:?}", err);
                Ok(Vec::new())
            }
        };
        let containers_get = response_result.unwrap_or_default();
        Some(containers_get.into_iter().filter(|c: &OrchestratorContainer| c.is_managed()).collect())
    }

    async fn container_start(&self, connector_id: String) -> () {
        let start_container_uri = format!("{}/{}/start", self.container_uri, connector_id);
        self.client.post(start_container_uri).send().await.unwrap();
    }

    async fn container_deploy(&self, connector: &Connector) -> Option<OrchestratorContainer> {
        let container_env_variables = connector.manager_contract_configuration.clone().unwrap()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect::<Vec<String>>();
        // 01. First operation, pull the image
        let create_image_uri = format!("{}/create?fromImage={}", self.image_uri, "opencti/connector-ipinfo:latest");
        let mut create_response = self.client.post(create_image_uri).send().await.unwrap();
        while let Some(_chunk) = create_response.chunk().await.unwrap() {} // Iter chunk to fetch all
        // 02. Deploy the container after success
        let deploy_container_uri = format!("{}/create?name={}", self.container_uri, "my-ipinfo-connector");
        let image_labels = HashMap::from([
            ("opencti-connector-id".into(), connector.id.clone().into_inner())
        ]);
        let json_body = PortainerDeployPayload {
            env: container_env_variables,
            image: "opencti/connector-ipinfo:latest".into(),
            labels: image_labels,
            host_config: PortainerDeployHostConfig {
                // RestartPolicy - will be managed by the composer
                network_mode: Some("opencti-dev_default".into())
            },
        };
        let deploy_response = self.client.post(deploy_container_uri)
            .json(&json_body).send().await.unwrap();
        let deploy_data: PortainerDeployResponse = deploy_response.json().await.unwrap();
        // 03. Return the result
        self.container(deploy_data.id.clone()).await
    }
}