use crate::api::{ApiConnector, ConnectorStatus};
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::Docker;
use bollard::auth::DockerCredentials;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::HostConfig;
use bollard::image::CreateImageOptions;
use futures::TryStreamExt;
use futures::future;
use std::collections::HashMap;
use tracing::{debug, error, info, trace};

impl DockerOrchestrator {
    pub fn new() -> Self {
        let docker = Docker::connect_with_socket_defaults().unwrap();
        Self { docker }
    }

    pub fn convert_labels(labels: Vec<String>) -> HashMap<String, String> {
        labels
            .iter()
            .map(|label| {
                let parts: Vec<&str> = label.split('=').collect();
                (parts[0].into(), parts[1].into())
            })
            .collect()
    }

    pub fn normalize_name(name: Option<String>) -> String {
        name.unwrap().strip_prefix("/").unwrap().into()
    }
}

#[async_trait]
impl Orchestrator for DockerOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let container_name = connector.container_name();
        let opts = Some(InspectContainerOptions::default());
        let container = self
            .docker
            .inspect_container(container_name.as_str(), opts)
            .await;
        match container {
            Ok(docker_container) => {
                let state = docker_container.state.unwrap();
                let restart_count = docker_container.restart_count.unwrap_or(0) as u32;
                let started_at = state.started_at;
                
                Some(OrchestratorContainer {
                    id: docker_container.id.unwrap(),
                    name: DockerOrchestrator::normalize_name(docker_container.name),
                    state: state.status.unwrap().to_string(),
                    envs: DockerOrchestrator::convert_labels(
                        docker_container.config.clone()?.env.unwrap(),
                    ),
                    labels: docker_container.config.clone()?.labels.unwrap(),
                    restart_count,
                    started_at,
                })
            },
            Err(e) => {
                debug!(
                    name = container_name,
                    error = %e,
                    "Container not found (this may be expected for new deployments)"
                );
                None
            }
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let manager_label = format!("opencti-manager={}", settings.manager.id.clone());
        let list_container_filters: HashMap<String, Vec<String>> =
            HashMap::from([("label".to_string(), Vec::from([manager_label]))]);

        let container_result = self
            .docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                filters: list_container_filters,
                ..Default::default()
            }))
            .await;
        match container_result {
            Ok(containers) => containers
                .into_iter()
                .map(|docker_container| {
                    let container_name: Option<String> =
                        docker_container.names.unwrap().first().cloned();
                    OrchestratorContainer {
                        id: docker_container.id.unwrap(),
                        name: DockerOrchestrator::normalize_name(container_name),
                        state: docker_container.state.unwrap(),
                        envs: HashMap::new(),
                        labels: docker_container.labels.unwrap(),
                        restart_count: 0, // Not available in list, will be updated by get()
                        started_at: None, // Not available in list, will be updated by get()
                    }
                })
                .collect(),
            Err(err) => {
                error!(
                    error = %err,
                    manager_id = %settings.manager.id,
                    "Failed to list managed containers"
                );
                Vec::new()
            }
        }
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        let container_name = connector.container_name();
        
        match self
            .docker
            .start_container(
                container_name.as_str(),
                None::<StartContainerOptions<String>>,
            )
            .await
        {
            Ok(_) => {
                debug!(name = container_name, "Container started");
            }
            Err(e) => {
                error!(
                    name = container_name,
                    error = %e,
                    "Failed to start container"
                );
            }
        }
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        let container_name = connector.container_name();
        
        match self
            .docker
            .stop_container(container_name.as_str(), None::<StopContainerOptions>)
            .await
        {
            Ok(_) => {
                debug!(name = container_name, "Container stopped");
            }
            Err(e) => {
                error!(
                    name = container_name,
                    error = %e,
                    "Failed to stop container"
                );
            }
        }
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        let container_name = container.name.as_str();
        let container_id = &container.id;
        
        match self
            .docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    v: true,
                    force: true,
                    link: false,
                }),
            )
            .await
        {
            Ok(_) => {
                debug!(
                    name = container_name,
                    id = container_id,
                    "Container removed"
                );
            }
            Err(e) => {
                error!(
                    name = container_name,
                    id = container_id,
                    error = %e,
                    "Failed to remove container"
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
        // Get settings and check for Docker options
        let settings = crate::settings();
        let docker_options = settings.opencti.daemon.docker.as_ref();
        
        // Get registry server URL if configured
        let registry_server = docker_options
            .and_then(|opts| opts.registry.as_ref())
            .and_then(|reg| reg.server.as_ref());
        
        // Build authentication credentials if configured
        let has_auth = docker_options
            .and_then(|opts| opts.registry.as_ref())
            .is_some();
        
        let auth = docker_options
            .and_then(|opts| opts.registry.as_ref())
            .map(|registry| DockerCredentials {
                username: registry.username.clone(),
                password: registry.password.clone(),
                auth: None,
                email: registry.email.clone(),
                serveraddress: registry.server.clone(),
                identitytoken: None,
                registrytoken: None,
            });

        // Construct the full image name with registry if configured
        let image_name = if let Some(registry) = registry_server {
            // Check if the image already has a registry prefix
            // (contains a dot before the first slash, like registry.com/image)
            let needs_prefix = if let Some(first_slash_pos) = connector.image.find('/') {
                // Check if there's a dot before the first slash (indicating a registry)
                !connector.image[..first_slash_pos].contains('.')
            } else {
                // No slash at all, it's just an image name
                true
            };
            
            if needs_prefix {
                // Add the registry prefix
                let full_image = format!("{}/{}", registry.trim_end_matches('/'), connector.image);
                debug!("Prefixing image with registry: {} -> {}", connector.image, full_image);
                full_image
            } else {
                // Image already has a registry prefix
                debug!("Image already has registry prefix: {}", connector.image);
                connector.image.clone()
            }
        } else {
            connector.image.clone()
        };

        // Log the pull attempt
        if registry_server.is_some() {
            info!("Starting pull of {}", image_name);
            if has_auth {
                debug!("Using authentication for registry pull");
            }
        } else {
            info!("Starting pull of {} from Docker Hub", image_name);
        }

        // Pull the image with optional authentication
        let deploy_response = self
            .docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: image_name.as_str(),
                    ..Default::default()
                }),
                None,
                auth,
            )
            .try_for_each(|info| {
                // Log pull progress at trace level to reduce verbosity
                if let Some(status) = info.status.as_deref() {
                    if let Some(progress) = info.progress.as_deref() {
                        trace!(
                            image = image_name,
                            status = status,
                            progress = progress,
                            "Image pull progress"
                        );
                    } else {
                        trace!(
                            image = image_name,
                            status = status,
                            "Image pull status"
                        );
                    }
                }
                
                // Log any error messages at error level
                if let Some(error) = info.error.as_deref() {
                    error!(
                        image = image_name,
                        error = error,
                        "Error during image pull"
                    );
                }
                
                future::ok(())
            })
            .await;
        match deploy_response {
            Ok(_) => {
                info!(
                    image = image_name,
                    "Image pulled successfully"
                );
                
                // Create the container
                let container_env_variables = connector
                    .container_envs()
                    .into_iter()
                    .map(|config| format!("{}={}", config.key, config.value))
                    .collect::<Vec<String>>();
                let labels = self.labels(connector);
                
                // Build host config with Docker options
                let mut host_config = HostConfig::default();
                
                if let Some(docker_opts) = docker_options {
                    // Apply Docker options to host config
                    if let Some(network_mode) = &docker_opts.network_mode {
                        host_config.network_mode = Some(network_mode.clone());
                    }
                    if let Some(extra_hosts) = &docker_opts.extra_hosts {
                        host_config.extra_hosts = Some(extra_hosts.clone());
                    }
                    if let Some(dns) = &docker_opts.dns {
                        host_config.dns = Some(dns.clone());
                    }
                    if let Some(dns_search) = &docker_opts.dns_search {
                        host_config.dns_search = Some(dns_search.clone());
                    }
                    if let Some(privileged) = docker_opts.privileged {
                        host_config.privileged = Some(privileged);
                    }
                    if let Some(cap_add) = &docker_opts.cap_add {
                        host_config.cap_add = Some(cap_add.clone());
                    }
                    if let Some(cap_drop) = &docker_opts.cap_drop {
                        host_config.cap_drop = Some(cap_drop.clone());
                    }
                    if let Some(security_opt) = &docker_opts.security_opt {
                        host_config.security_opt = Some(security_opt.clone());
                    }
                    if let Some(userns_mode) = &docker_opts.userns_mode {
                        host_config.userns_mode = Some(userns_mode.clone());
                    }
                    if let Some(pid_mode) = &docker_opts.pid_mode {
                        host_config.pid_mode = Some(pid_mode.clone());
                    }
                    if let Some(ipc_mode) = &docker_opts.ipc_mode {
                        host_config.ipc_mode = Some(ipc_mode.clone());
                    }
                    if let Some(uts_mode) = &docker_opts.uts_mode {
                        host_config.uts_mode = Some(uts_mode.clone());
                    }
                    if let Some(runtime) = &docker_opts.runtime {
                        host_config.runtime = Some(runtime.clone());
                    }
                    if let Some(shm_size) = docker_opts.shm_size {
                        host_config.shm_size = Some(shm_size);
                    }
                    if let Some(sysctls) = &docker_opts.sysctls {
                        host_config.sysctls = Some(sysctls.clone());
                    }
                    if let Some(ulimits) = &docker_opts.ulimits {
                        // Convert ulimits from HashMap to bollard's expected format
                        let ulimits_vec: Vec<bollard::models::ResourcesUlimits> = ulimits.iter()
                            .filter_map(|ulimit_map| {
                                if let (Some(name), Some(soft), Some(hard)) = (
                                    ulimit_map.get("name").and_then(|v| v.as_str()),
                                    ulimit_map.get("soft").and_then(|v| v.as_i64()),
                                    ulimit_map.get("hard").and_then(|v| v.as_i64()),
                                ) {
                                    Some(bollard::models::ResourcesUlimits {
                                        name: Some(name.to_string()),
                                        soft: Some(soft),
                                        hard: Some(hard),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !ulimits_vec.is_empty() {
                            host_config.ulimits = Some(ulimits_vec);
                        }
                    }
                }
                
                let config = Config {
                    image: Some(image_name.clone()),
                    env: Some(container_env_variables),
                    labels: Some(labels),
                    host_config: Some(host_config),
                    ..Default::default()
                };

                let container_name = connector.container_name();
                match self
                    .docker
                    .create_container::<String, String>(
                        Some(CreateContainerOptions {
                            name: container_name.clone(),
                            platform: None,
                        }),
                        config,
                    )
                    .await
                {
                    Ok(_) => {
                        info!(
                            name = container_name,
                            image = image_name,
                            "Container created successfully"
                        );
                        
                        // Get the created container
                        let created = self.get(connector).await;
                        
                        // Start the container if needed
                        let is_starting = connector.requested_status.clone().eq("starting");
                        if is_starting && created.is_some() {
                            self.start(&created.clone().unwrap(), connector).await;
                        }
                        
                        created
                    }
                    Err(e) => {
                        error!(
                            name = container_name,
                            image = image_name,
                            error = %e,
                            "Failed to create container"
                        );
                        
                        // Provide helpful error context
                        if e.to_string().contains("Conflict") {
                            error!("Container with name '{}' already exists. Consider removing it first.", container_name);
                        } else if e.to_string().contains("No such image") {
                            error!("Image '{}' was pulled but cannot be found. This may indicate a Docker daemon issue.", image_name);
                        }
                        
                        None
                    }
                }
            }
            Err(err) => {
                // Detailed error logging
                if let Some(registry) = registry_server {
                    error!(
                        "Failed to pull image '{}' (resolved as '{}') from registry '{}'. Error: {:?}", 
                        connector.image,
                        image_name, 
                        registry,
                        err
                    );
                    // Provide helpful hints
                    error!("Possible causes: 1) Registry URL is incorrect, 2) Authentication failed, 3) Image doesn't exist on registry");
                    if !has_auth {
                        error!("Note: No authentication configured for registry. Check docker.registry configuration.");
                    }
                } else {
                    error!(
                        "Failed to pull image '{}' from Docker Hub. Error: {:?}", 
                        image_name,
                        err
                    );
                    error!("Possible causes: 1) Image doesn't exist, 2) Network issues, 3) Docker Hub rate limiting");
                }
                None
            }
        }
    }

    async fn logs(
        &self,
        _container: &OrchestratorContainer,
        connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        let opts = Some(LogsOptions::<String> {
            follow: false,
            stdout: true,
            stderr: true,
            tail: "100".to_string(),
            ..Default::default()
        });
        let logs = self.docker.logs(connector.container_name().as_str(), opts);
        let mut logs_content = Vec::new();
        logs.try_for_each(|log| {
            logs_content.push(log.to_string());
            future::ok(())
        })
        .await
        .unwrap();
        Some(logs_content)
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" => ConnectorStatus::Started,
            _ => ConnectorStatus::Stopped,
        }
    }
}
