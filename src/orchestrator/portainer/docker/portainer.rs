use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Portainer;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::portainer::docker::{
    PortainerDeployHostConfig, PortainerDeployPayload, PortainerDeployResponse,
    PortainerDockerOrchestrator, PortainerGetResponse,
};
use crate::orchestrator::registry_resolver::RegistryResolver;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use base64::Engine as _;
use bollard::models::ContainerSummary;
use header::HeaderValue;
use k8s_openapi::serde_json;
use reqwest::header::HeaderMap;
use reqwest::{Client, header};
use std::collections::HashMap;
use std::fmt::Error;
use tracing::{debug, error, info, trace, warn};

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
        let header_value = match HeaderValue::from_bytes(config.api_key.as_bytes()) {
            Ok(value) => value,
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to create header value from API key"
                );
                panic!("Cannot initialize Portainer orchestrator with invalid API key");
            }
        };
        headers.insert(X_API_KEY, header_value);
        
        let client = match Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to build Portainer HTTP client"
                );
                panic!("Cannot initialize Portainer orchestrator without HTTP client");
            }
        };
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
        
        let serialized_filter = match serde_json::to_string(&filter) {
            Ok(filter) => filter,
            Err(e) => {
                error!(
                    error = %e,
                    manager_id = %settings.manager.id,
                    "Failed to serialize filter for Portainer list"
                );
                return Vec::new();
            }
        };
        
        let list_uri = format!(
            "{}/json?all=true&filters={}",
            self.container_uri, serialized_filter
        );
        let response = self.client.get(list_uri.clone()).send().await;
        let response_result: Result<Vec<OrchestratorContainer>, _> = match response {
            Ok(data) => {
                match data.json::<Vec<ContainerSummary>>().await {
                    Ok(response) => {
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
                    Err(e) => {
                        error!(
                            error = %e,
                            "Failed to parse Portainer container list response"
                        );
                        Ok(Vec::new())
                    }
                }
            }
            Err(e) => {
                error!(
                    error = %e,
                    manager_id = %settings.manager.id,
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

    async fn start(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        let start_container_uri = format!("{}/{}/start", self.container_uri, container.id);
        match self.client.post(start_container_uri).send().await {
            Ok(_) => {
                debug!(
                    container_id = container.id,
                    name = container.name,
                    "Container started via Portainer"
                );
            }
            Err(e) => {
                error!(
                    container_id = container.id,
                    name = container.name,
                    error = %e,
                    "Failed to start container via Portainer"
                );
            }
        }
    }

    async fn stop(&self, container: &OrchestratorContainer, _connector: &ApiConnector) -> () {
        let stop_container_uri = format!("{}/{}/stop", self.container_uri, container.id);
        match self.client.post(stop_container_uri).send().await {
            Ok(_) => {
                debug!(
                    container_id = container.id,
                    name = container.name,
                    "Container stopped via Portainer"
                );
            }
            Err(e) => {
                error!(
                    container_id = container.id,
                    name = container.name,
                    error = %e,
                    "Failed to stop container via Portainer"
                );
            }
        }
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
        // Get registry configuration from daemon level
        let settings = crate::settings();
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = RegistryResolver::new(registry_config.clone());

        // Resolve image name with registry prefix if needed
        let resolved_image = match resolver.resolve_image(&connector.image) {
            Ok(resolved) => {
                debug!(
                    orchestrator = "portainer",
                    original_image = connector.image,
                    resolved_image = resolved.full_name,
                    needs_auth = resolved.needs_auth,
                    "Image resolved for Portainer deployment"
                );
                resolved
            }
            Err(e) => {
                error!(
                    orchestrator = "portainer",
                    image = connector.image,
                    error = %e,
                    "Failed to resolve image"
                );
                return None;
            }
        };

        let create_image_uri = format!(
            "{}/create?fromImage={}",
            self.image_uri,
            resolved_image.full_name
        );

        let mut pull_headers = HeaderMap::new();
        if resolved_image.needs_auth {
            match resolver.get_docker_credentials() {
                Ok(Some(credentials)) => {
                    info!(
                        orchestrator = "portainer",
                        operation = "auth",
                        status = "completed",
                        "Registry authentication completed"
                    );
                    
                    let auth_config = serde_json::json!({
                        "username": credentials.username,
                        "password": credentials.password,
                        "auth": "",
                        "serveraddress": credentials.serveraddress
                    });
                    
                    let auth_string = base64::engine::general_purpose::STANDARD
                        .encode(auth_config.to_string());
                    
                    if let Ok(header_value) = HeaderValue::from_str(&auth_string) {
                        pull_headers.insert("X-Registry-Auth", header_value);
                    } else {
                        error!("Failed to create auth header");
                    }
                }
                Ok(None) => {
                    warn!(
                        orchestrator = "portainer",
                        operation = "auth",
                        "No credentials available, attempting pull without authentication"
                    );
                }
                Err(e) => {
                    error!(
                        orchestrator = "portainer",
                        operation = "auth",
                        status = "failed",
                        error = %e,
                        "Registry authentication failed"
                    );
                    warn!("Attempting pull without authentication");
                }
            }
        }

        if let Some(registry_server) = &resolved_image.registry_server {
            info!(
                orchestrator = "portainer",
                image = resolved_image.full_name,
                registry = registry_server,
                operation = "pull",
                status = "started",
                "Starting image pull from private registry"
            );
        } else {
            info!(
                orchestrator = "portainer",
                image = resolved_image.full_name,
                operation = "pull",
                status = "started",
                "Starting image pull from Docker Hub"
            );
        }

        let pull_request = if pull_headers.is_empty() {
            self.client.post(&create_image_uri)
        } else {
            self.client.post(&create_image_uri).headers(pull_headers)
        };

        let mut create_response = match pull_request.send().await {
            Ok(response) => response,
            Err(e) => {
                error!(
                    orchestrator = "portainer",
                    image = resolved_image.full_name,
                    operation = "pull",
                    status = "failed",
                    error = %e,
                    "Image pull failed via Portainer"
                );
                return None;
            }
        };
        
        loop {
            match create_response.chunk().await {
                Ok(Some(chunk)) => {
                    trace!(
                        orchestrator = "portainer",
                        image = resolved_image.full_name,
                        chunk_size = chunk.len(),
                        "Processing image pull chunk"
                    );
                }
                Ok(None) => {
                    info!(
                        orchestrator = "portainer",
                        image = resolved_image.full_name,
                        operation = "pull",
                        status = "completed",
                        "Image pull completed"
                    );
                    break;
                }
                Err(e) => {
                    error!(
                        orchestrator = "portainer",
                        image = resolved_image.full_name,
                        operation = "pull",
                        status = "failed",
                        error = %e,
                        "Error processing image pull chunk"
                    );
                    return None;
                }
            }
        }

        let image_name: String = connector.container_name();
        let deploy_container_uri = format!("{}/create?name={}", self.container_uri, image_name);
        let mut image_labels = self.labels(connector);
        let portainer_config = self.config.clone();
        if let Some(stack_label) = portainer_config.stack {
            image_labels.insert("com.docker.compose.project".to_string(), stack_label);
        }
        let env_vars = connector.container_envs();
        let container_envs = env_vars
            .iter()
            .map(|config| format!("{}={}", config.key, config.value.as_str()))
            .collect();
        let image_name_for_deploy = resolved_image.full_name.clone();
        let json_body = PortainerDeployPayload {
            env: container_envs,
            image: image_name_for_deploy.clone(), // Use resolved image name
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
                match response.json::<PortainerDeployResponse>().await {
                    Ok(deploy_data) => {
                        info!(
                            orchestrator = "portainer",
                            image = image_name_for_deploy,
                            operation = "deploy",
                            status = "completed",
                            container_id = deploy_data.id,
                            "Container deployment completed"
                        );
                        self.get(connector).await
                    }
                    Err(e) => {
                        error!(
                            orchestrator = "portainer",
                            image = image_name_for_deploy,
                            operation = "deploy",
                            status = "failed",
                            error = %e,
                            "Failed to parse deployment response"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                error!(
                    orchestrator = "portainer",
                    image = image_name_for_deploy,
                    operation = "deploy",
                    status = "failed",
                    error = %e,
                    "Container deployment failed"
                );
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
        
        let logs_response = match self.client.get(logs_container_uri).send().await {
            Ok(response) => response,
            Err(e) => {
                error!(
                    container_id = container.id,
                    name = container.name,
                    error = %e,
                    "Failed to fetch logs from Portainer"
                );
                return None;
            }
        };
        
        match logs_response.text().await {
            Ok(text_logs) => {
                Some(text_logs.lines().map(|line| line.to_string()).collect())
            }
            Err(e) => {
                error!(
                    container_id = container.id,
                    name = container.name,
                    error = %e,
                    "Failed to parse logs response from Portainer"
                );
                None
            }
        }
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" => ConnectorStatus::Started,
            _ => ConnectorStatus::Stopped,
        }
    }
}
