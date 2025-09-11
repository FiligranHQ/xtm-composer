use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Portainer;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::portainer::docker::{
    PortainerDeployHostConfig, PortainerDeployPayload, PortainerDeployResponse,
    PortainerDockerOrchestrator, PortainerGetResponse,
};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::models::ContainerSummary;
use header::HeaderValue;
use k8s_openapi::serde_json;
use reqwest::header::HeaderMap;
use reqwest::{Client, header};
use std::collections::HashMap;
use std::fmt::Error;
use tracing::{debug, error, info};

const X_API_KEY: &str = "X-API-KEY";

impl PortainerDockerOrchestrator {
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
impl Orchestrator for PortainerDockerOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let get_uri = format!("{}/{}/json", self.container_uri, connector.container_name());
        let response = self.client.get(get_uri).send().await;
        let response_result: Result<Option<PortainerGetResponse>, _> = match response {
            Ok(data) => data.json().await,
            Err(err) => {
                error!(
                    error = err.to_string(),
                    "Portainer error fetching containers"
                );
                Ok(None)
            }
        };
        let container_get = response_result.unwrap_or_default();
        if container_get.is_some() {
            let response_data = container_get.unwrap();
            let container_envs: HashMap<String, String> = response_data
                .config
                .env
                .iter()
                .map(|env| {
                    let parts: Vec<&str> = env.split('=').collect();
                    (parts[0].into(), parts[1].into())
                })
                .collect();
            Some(OrchestratorContainer {
                id: response_data.id,
                name: response_data.name,
                state: response_data.state.status.clone(),
                labels: response_data.config.labels,
                envs: container_envs,
                restart_count: response_data.restart_count.unwrap_or(0) as u32,
                started_at: response_data.state.started_at,
            })
        } else {
            None
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let mut label_filters = Vec::new();
        label_filters.push(format!("opencti-manager={}", settings.manager.id.clone()));
        let filter: HashMap<String, Vec<String>> = HashMap::from([("label".into(), label_filters)]);
        let serialized_filter = serde_json::to_string(&filter).unwrap();
        let list_uri = format!(
            "{}/json?all=true&filters={}",
            self.container_uri, serialized_filter
        );
        let response = self.client.get(list_uri.clone()).send().await;
        let response_result: Result<Vec<OrchestratorContainer>, _> = match response {
            Ok(data) => {
                let response: Vec<ContainerSummary> = data.json().await.unwrap();
                let containers = response
                    .into_iter()
                    .map(|summary| {
                        let container_name: Option<String> =
                            summary.names.unwrap().first().cloned();
                        OrchestratorContainer {
                            id: summary.id.unwrap(),
                            name: DockerOrchestrator::normalize_name(container_name),
                            state: summary.state.unwrap(),
                            envs: HashMap::new(),
                            labels: summary.labels.unwrap(),
                            restart_count: 0, // Not available in list, will be updated by get()
                            started_at: None, // Not available in list, will be updated by get()
                        }
                    })
                    .collect();
                Ok::<Vec<OrchestratorContainer>, Error>(containers)
            }
            Err(err) => {
                error!(
                    error = err.to_string(),
                    "Portainer error fetching containers"
                );
                Ok(Vec::new())
            }
        };
        let containers_get = response_result.unwrap_or_default();
        containers_get
            .into_iter()
            .filter(|c: &OrchestratorContainer| c.is_managed())
            .collect()
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
        let container_name = container.name.as_str();
        let delete_container_uri =
            format!("{}/{}?v=0&force=true", self.container_uri, container.id);
        let remove_response = self.client.delete(delete_container_uri).send().await;
        match remove_response {
            Ok(_) => {
                info!(name = container_name, "Removed container");
            }
            Err(err) => {
                error!(
                    name = container_name,
                    error = err.to_string(),
                    "Could not remove container"
                );
            }
        }
    }

    async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        // Remove the current container if needed
        let container = self.get(connector).await;
        if container.is_some() {
            let _ = self.remove(&container.unwrap()).await;
        }
        // Deploy the new one
        self.deploy(connector).await
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

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" => ConnectorStatus::Started,
            _ => ConnectorStatus::Stopped,
        }
    }
}
