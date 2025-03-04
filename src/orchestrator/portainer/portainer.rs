use crate::api::ApiConnector;
use crate::api::opencti::connector::ConnectorCurrentStatus;
use crate::config::settings::{Portainer};
use crate::orchestrator::portainer::{
    PortainerDeployHostConfig, PortainerDeployPayload, PortainerDeployResponse,
    PortainerGetResponse, PortainerOrchestrator,
};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use header::HeaderValue;
use k8s_openapi::serde_json;
use reqwest::header::HeaderMap;
use reqwest::{Client, header};
use std::collections::HashMap;
use tracing::{debug, error};

const X_API_KEY: &str = "X-API-KEY";

impl PortainerOrchestrator {
    pub fn new(config: Portainer) -> Self {
        let container_uri = format!(
            "{}/api/endpoints/{}/docker/{}/containers",
            config.api, config.env_id, config.api_version
        );
        let image_uri = format!(
            "{}/api/endpoints/{}/docker/{}/images",
            config.api, config.env_id, config.api_version
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            X_API_KEY,
            HeaderValue::from_bytes(config.api_key.as_bytes()).unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        Self {
            image_uri,
            container_uri,
            client,
            config,
        }
    }
}

#[async_trait]
impl Orchestrator for PortainerOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let mut label_filters = Vec::new();
        label_filters.push(format!("opencti-connector-id={}", connector.id.clone()));
        let filter: HashMap<String, Vec<String>> = HashMap::from([("label".into(), label_filters)]);
        let serialized_filter = serde_json::to_string(&filter).unwrap();
        let list_uri = format!(
            "{}/json?all=true&filters={}",
            self.container_uri, serialized_filter
        );
        let response = self.client.get(list_uri).send().await;
        let response_result: Result<Vec<PortainerGetResponse>, _> = match response {
            Ok(data) => data.json().await,
            Err(err) => {
                error!(
                    error = err.to_string(),
                    "Portainer error fetching containers"
                );
                Ok(Vec::new())
            }
        };
        let containers_get = response_result.unwrap_or_default();
        if !containers_get.is_empty() {
            let response_data = containers_get.into_iter().next().unwrap();
            let container_envs: HashMap<String, String> = response_data
                .config
                .env
                .iter()
                .map(|env| {
                    let parts: Vec<&str> = env.split(',').collect();
                    (parts[0].into(), parts[1].into())
                })
                .collect();
            Some(OrchestratorContainer {
                id: response_data.id,
                state: response_data.state.status,
                labels: response_data.config.labels,
                envs: container_envs,
                // image: response_data.image,
            })
        } else {
            None
        }
    }

    async fn list(&self) -> Option<Vec<OrchestratorContainer>> {
        let settings = crate::settings();
        let mut label_filters = Vec::new();
        label_filters.push(format!("opencti-manager={}", settings.manager.id.clone()));
        let filter: HashMap<String, Vec<String>> = HashMap::from([("label".into(), label_filters)]);
        let serialized_filter = serde_json::to_string(&filter).unwrap();
        let list_uri = format!(
            "{}/json?all=true&filters={}",
            self.container_uri, serialized_filter
        );
        let response = self.client.get(list_uri).send().await;
        let response_result = match response {
            Ok(data) => data.json().await,
            Err(err) => {
                error!(
                    error = err.to_string(),
                    "Portainer error fetching containers"
                );
                Ok(Vec::new())
            }
        };
        let containers_get = response_result.unwrap_or_default();
        Some(
            containers_get
                .into_iter()
                .filter(|c: &OrchestratorContainer| c.is_managed())
                .collect(),
        )
    }

    async fn start(&self, container: &OrchestratorContainer, _connector: &ApiConnector) -> () {
        let start_container_uri = format!("{}/{}/start", self.container_uri, container.id);
        self.client.post(start_container_uri).send().await.unwrap();
    }

    async fn stop(&self, container: &OrchestratorContainer, _connector: &ApiConnector) -> () {
        let start_container_uri = format!("{}/{}/stop", self.container_uri, container.id);
        self.client.post(start_container_uri).send().await.unwrap();
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        let delete_container_uri =
            format!("{}/{}?v=0&force=true", self.container_uri, container.id);
        self.client
            .delete(delete_container_uri)
            .send()
            .await
            .unwrap();
    }

    async fn refresh(&self, _connector: &ApiConnector) -> Option<OrchestratorContainer> {
        todo!("portainer refresh")
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        // region First operation, pull the image
        let create_image_uri = format!(
            "{}/create?fromImage={}",
            self.image_uri,
            connector.image.clone()
        );
        let mut create_response = self.client.post(create_image_uri).send().await.unwrap();
        while let Some(_chunk) = create_response.chunk().await.unwrap() {} // Iter chunk to fetch all
        // endregion
        // region Deploy the container after success
        let image_name: String = connector.container_name();
        let deploy_container_uri = format!("{}/create?name={}", self.container_uri, image_name);
        let mut image_labels = self.labels(connector);
        let portainer_config = self.config.clone();
        if portainer_config.stack.is_some() {
            let stack_label = portainer_config.stack.unwrap();
            image_labels.insert("com.docker.compose.project".to_string(), stack_label);
        }
        let env_vars = connector.container_envs();
        let container_envs = env_vars
            .iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect();
        let json_body = PortainerDeployPayload {
            env: container_envs,
            image: connector.image.clone(),
            labels: image_labels,
            host_config: PortainerDeployHostConfig {
                network_mode: portainer_config.network_mode,
            },
        };
        let deploy_response = self
            .client
            .post(deploy_container_uri)
            .json(&json_body)
            .send()
            .await;
        match deploy_response {
            Ok(response) => {
                let deploy_data: PortainerDeployResponse = response.json().await.unwrap();
                debug!(id = deploy_data.id, "Portainer container deployed");
                self.get(connector).await
            }
            Err(err) => {
                error!(error = err.to_string(), "Error deploying the container");
                None
            }
        }
    }

    async fn logs(
        &self,
        container: &OrchestratorContainer,
        _connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        let logs_container_uri = format!(
            "{}/{}/logs?stderr=1&stdout=1&tail=100",
            self.container_uri, container.id
        );
        let logs_response = self.client.get(logs_container_uri).send().await.unwrap();
        let text_logs = logs_response.text().await.unwrap();
        Some(text_logs.lines().map(|line| line.to_string()).collect())
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus {
        match container.state.as_str() {
            "running" => ConnectorCurrentStatus::Started,
            "exited" => ConnectorCurrentStatus::Stopped,
            _ => ConnectorCurrentStatus::Stopped,
        }
    }
}
