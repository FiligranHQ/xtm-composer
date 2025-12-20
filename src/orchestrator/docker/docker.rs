use crate::api::{ApiConnector, ConnectorStatus};
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::HostConfig;
use bollard::image::CreateImageOptions;
use futures::TryStreamExt;
use futures::future;
use std::collections::HashMap;
use tracing::{debug, error, info};

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

    /// Get full image name with registry prefix if configured
    ///
    /// This method intelligently prepends the configured registry URL to image names
    /// that don't already have a registry prefix.
    ///
    /// # Examples
    /// ```
    /// // Config: registry.url = "localhost:5000"
    ///
    /// // Simple image name → Prepend registry
    /// "connector-misp:5.0.0" → "localhost:5000/connector-misp:5.0.0"
    ///
    /// // Organization/image format → Prepend registry
    /// "myorg/connector:1.0" → "localhost:5000/myorg/connector:1.0"
    ///
    /// // Already has registry → Keep as-is
    /// "docker.io/alpine:3.18" → "docker.io/alpine:3.18"
    /// "registry.company.com/app:v1" → "registry.company.com/app:v1"
    /// ```
    ///
    /// # Detection Logic
    /// An image is considered to have a registry prefix if:
    /// - It contains '/' AND the first part contains '.' (e.g., "docker.io", "localhost:5000")
    ///
    /// # Arguments
    /// * `image` - Original image name from OpenCTI (e.g., "connector-misp:5.0.0")
    ///
    /// # Returns
    /// * Full image name, optionally prefixed with registry URL
    fn get_full_image_name(&self, image: &str) -> String {
        // Step 1: Load global settings from config/default.yaml
        let settings = crate::settings();
        let docker_config = settings.opencti.daemon.docker.as_ref();

        // Step 2: Check if Docker orchestrator configuration exists
        if let Some(docker_opts) = docker_config {

            // Step 3: Check if registry configuration exists
            if let Some(registry_config) = &docker_opts.registry {

                // Step 4: Check if registry URL is configured
                if let Some(registry_url) = &registry_config.url {

                    // Step 5: Determine if we should prepend the registry URL
                    // Decision criteria:
                    //   - If image has NO '/', it's a simple name (e.g., "alpine:3.18") → Prepend
                    //   - If image has '/' but first part has NO '.', it's org/image format (e.g., "myorg/app") → Prepend
                    //   - If image has '/' and first part has '.', it already has a registry (e.g., "docker.io/alpine") → Don't prepend
                    if !image.contains('/') || !image.split('/').next().unwrap().contains('.') {
                        // Image doesn't have a registry prefix → Prepend our registry URL
                        // trim_end_matches('/') removes trailing slashes to avoid "localhost:5000//image"
                        return format!("{}/{}", registry_url.trim_end_matches('/'), image);
                    }
                    // If we reach here: Image already has a registry prefix → Use as-is
                }
            }
        }

        // Step 6: No registry configured or image already has registry → Return unchanged
        image.to_string()
    }

    /// Build Docker registry authentication credentials
    ///
    /// This method constructs authentication credentials for private Docker registries.
    /// If no credentials are configured, it returns None and Docker will fall back to:
    /// - Public access (no authentication)
    /// - Credentials from ~/.docker/config.json
    /// - Docker daemon's credential store
    ///
    /// # Examples
    /// ```
    /// // Config with credentials:
    /// registry:
    ///   url: localhost:5000
    ///   username: username
    ///   password: secret123
    ///
    /// // Returns: Some(DockerCredentials { username: "username", password: "secret123" })
    ///
    /// // Config without credentials:
    /// registry:
    ///   url: localhost:5000
    ///
    /// // Returns: Some(DockerCredentials { username: None, password: None })
    ///
    /// // No registry config at all:
    /// // Returns: None (Docker will use default authentication)
    /// ```
    ///
    /// # Returns
    /// * `Some(DockerCredentials)` - If registry configuration exists (credentials optional)
    /// * `None` - If no registry configuration found
    fn build_registry_auth(&self) -> Option<bollard::auth::DockerCredentials> {
        // Step 1: Load global settings from config/default.yaml
        let settings = crate::settings();

        // Step 2: Get Docker orchestrator configuration
        // The '?' operator returns None early if docker config doesn't exist
        let docker_config = settings.opencti.daemon.docker.as_ref()?;

        // Step 3: Get registry configuration
        // The '?' operator returns None early if registry config doesn't exist
        let registry_config = docker_config.registry.as_ref()?;

        // Step 4: Build credentials struct
        // Note: username and password are Option<String>, so they can be None
        // If they're None, Docker will attempt unauthenticated access or use daemon credentials
        Some(bollard::auth::DockerCredentials {
            username: registry_config.username.clone(),  // Optional: Can be None
            password: registry_config.password.clone(),  // Optional: Can be None
            ..Default::default()  // Use default values
        })
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
            Err(_) => {
                debug!(name = container_name, "Could not find docker container");
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
                error!(error = err.to_string(), "Error fetching containers");
                Vec::new()
            }
        }
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        let container_name = connector.container_name();
        let _ = self
            .docker
            .start_container(
                container_name.as_str(),
                None::<StartContainerOptions<String>>,
            )
            .await;
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        let container_name = connector.container_name();
        let _ = self
            .docker
            .stop_container(container_name.as_str(), None::<StopContainerOptions>)
            .await;
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        let container_name = container.name.as_str();
        let remove_response = self
            .docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    v: true,
                    force: true,
                    link: false,
                }),
            )
            .await;
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
        // Step 1: Build full image name with registry prefix (if configured)
        // Example: "connector-misp:5.0.0" → "localhost:5000/connector-misp:5.0.0"
        let full_image_name = self.get_full_image_name(&connector.image);

        // Step 2: Build authentication credentials (if configured)
        // Returns:
        //   - Some(credentials) if registry config exists with username/password
        //   - None if no registry config (will use Docker's default auth)
        let auth = self.build_registry_auth();

        // Step 3: Log deployment information for debugging purpose only
        let settings = crate::settings();
        if let Some(docker_config) = settings.opencti.daemon.docker.as_ref() {
            if let Some(registry_config) = &docker_config.registry {
                if let Some(registry_url) = &registry_config.url {
                    // Log: Custom registry with URL
                    info!(
                        "Deploying container - Registry: {}, Original image: {}, Pulling: {}",
                        registry_url,
                        connector.image,
                        full_image_name
                    );
                } else {
                    // Log: Registry config exists but no URL
                    info!("Deploying container - Pulling image: {}", full_image_name);
                }
            } else {
                // Log: No registry config (using Docker Hub or daemon defaults)
                info!("Deploying container - Pulling image: {} (default registry)", full_image_name);
            }
        } else {
            // Log: No Docker config at all
            info!("Deploying container - Pulling image: {} (default registry)", full_image_name);
        }

        // Pull the image from the registry (with optional authentication)
        // This is an async streaming operation that reports progress
        let deploy_response = self
            .docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: full_image_name.as_str(),  // Full image name with registry
                    ..Default::default()
                }),
                None,  // No rootfs changes
                auth,  // Optional authentication (can be None)
            )
            .try_for_each(|info| {
                info!(
                    "{} {:?} pulling...",
                    full_image_name.as_str(),
                    info.status.as_deref()
                );
                future::ok(())
            })
            .await;
        match deploy_response {
            Ok(_) => {
                // Create the container
                let container_env_variables = connector
                    .container_envs()
                    .into_iter()
                    .map(|config| format!("{}={}", config.key, config.value))
                    .collect::<Vec<String>>();
                let labels = self.labels(connector);
                
                // Build host config with Docker options
                let mut host_config = HostConfig::default();
                
                // Get settings and check for Docker options
                let settings = crate::settings();
                let docker_options = settings.opencti.daemon.docker.as_ref();
                
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
                    image: Some(full_image_name.clone()),
                    env: Some(container_env_variables),
                    labels: Some(labels),
                    host_config: Some(host_config),
                    ..Default::default()
                };

                let create_response = self
                    .docker
                    .create_container::<String, String>(
                        Some(CreateContainerOptions {
                            name: connector.container_name(),
                            platform: None,
                        }),
                        config,
                    )
                    .await;
                match create_response {
                    Ok(_) => {}
                    Err(err) => {
                        error!(error = err.to_string(), "Error creating container");
                    }
                }

                // Get the created connector
                let created = self.get(connector).await;
                // Start the container if needed
                let is_starting = connector.requested_status.clone().eq("starting");
                if is_starting {
                    self.start(&created.clone().unwrap(), connector).await;
                }
                // Return the created container
                created
            }
            Err(_) => {
                error!(image = connector.image, "Error fetching container image");
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
