use async_trait::async_trait;
use header::HeaderValue;
use reqwest::{header, Client};
use reqwest::header::HeaderMap;
use crate::config::settings::Portainer;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};

const X_API_KEY: &str = "X-API-KEY";

#[derive(Default)]
pub struct PortainerOrchestrator {
    base_uri: String,
    client: Client
}

impl PortainerOrchestrator {
    pub fn new(config: &Portainer) -> Self {
        let base_uri = format!("{}/api/endpoints/{}/docker/{}/containers",
                               config.api, config.env_id, config.api_version);
        let mut headers = HeaderMap::new();
        headers.insert(X_API_KEY, HeaderValue::from_bytes(config.api_key.as_bytes()).unwrap());
        let client = Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build().unwrap();
        Self { base_uri, client }
    }
}

// GET https://localhost:9443/api/endpoints/3/docker/v1.41/containers/json?all=true
// POST https://localhost:9443/api/endpoints/3/docker/v1.41/containers/create?name=test
// POST https://localhost:9443/api/endpoints/3/docker/v1.41/containers/803eb3a4fa131d2823c0d9ae78368d51326445744e55d6441446b4ccb6b415d1/start
// DEL https://localhost:9443/api/endpoints/3/docker/v1.41/containers/803eb3a4fa131d2823c0d9ae78368d51326445744e55d6441446b4ccb6b415d1?v=0&force=true
#[async_trait]
impl Orchestrator for PortainerOrchestrator {
    async fn containers(&self) -> Vec<OrchestratorContainer> {
        let list_uri = format!("{}/json?all=true", self.base_uri);
        let containers: Vec<OrchestratorContainer> = self.client.get(list_uri)
            .send().await.unwrap()
            .json().await.unwrap();
        containers.into_iter().filter(|c| c.is_managed()).collect()
    }
}