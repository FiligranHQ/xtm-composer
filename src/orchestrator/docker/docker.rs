use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::{Daemon, Docker as DockerOptions, Registry};
use crate::orchestrator::docker::compat::{self, CompatApi};
use crate::orchestrator::docker::{ContainerEngine, DockerOrchestrator};
use crate::orchestrator::image::Image;
use crate::api::PROXY_CA_CERT_MOUNT_PATH;
use crate::orchestrator::ensure_proxy_ca_file;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::auth::DockerCredentials;
use bollard::errors::Error as BollardError;
use bollard::{API_DEFAULT_VERSION, Docker};

use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, InspectContainerOptions, ListContainersOptions,
    LogsOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use futures::TryStreamExt;
use futures::future;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, error, info};

/// Request timeout of the bollard client, in seconds.
pub(crate) const CLIENT_TIMEOUT: u64 = 120;
#[cfg(unix)]
const DEFAULT_DOCKER_SOCKET: &str = "/var/run/docker.sock";

impl DockerOrchestrator {
    /// Connect to the Docker API socket, waiting for the API to answer.
    pub async fn new(daemon: &Daemon, retry_interval: Duration) -> Self {
        let options = daemon.docker.clone();
        let socket_path = options.as_ref().and_then(|options| options.socket_path.clone());
        let docker = match &socket_path {
            Some(path) => Docker::connect_with_socket(path, CLIENT_TIMEOUT, API_DEFAULT_VERSION),
            None => Docker::connect_with_socket_defaults(),
        }
        .unwrap_or_else(|err| panic!("Cannot connect to the Docker socket: {}", err));
        // Docker rejects the requests of a client more recent than itself
        let docker = loop {
            match docker.clone().negotiate_version().await {
                Ok(negotiated) => break negotiated,
                Err(err) => {
                    error!(
                        error = err.to_string(),
                        retry_in_seconds = retry_interval.as_secs(),
                        "Cannot query the Docker API: check that the daemon is running and that the Composer user can read and write its socket"
                    );
                    tokio::time::sleep(retry_interval).await;
                }
            }
        };
        info!(
            socket = socket_path.as_deref().unwrap_or("default"),
            api_version = docker.client_version().to_string(),
            "Connected to Docker"
        );
        #[cfg(unix)]
        let compat = CompatApi::new(
            socket_path.as_deref().unwrap_or(DEFAULT_DOCKER_SOCKET),
            Some(docker.client_version().to_string()),
        )
        .ok();
        #[cfg(not(unix))]
        let compat = None;
        Self::with_engine(docker, ContainerEngine::Docker, compat, options, daemon.registry.clone())
    }

    pub(crate) fn with_engine(
        docker: Docker,
        engine: ContainerEngine,
        compat: Option<CompatApi>,
        options: Option<DockerOptions>,
        registry: Option<Registry>,
    ) -> Self {
        Self {
            docker,
            engine,
            compat,
            options,
            registry,
        }
    }

    pub fn convert_labels(labels: Vec<String>) -> HashMap<String, String> {
        labels
            .iter()
            .filter_map(|label| label.split_once('='))
            .map(|(key, value)| (key.into(), value.into()))
            .collect()
    }

    pub fn normalize_name(name: Option<String>) -> String {
        name.unwrap_or_default().trim_start_matches('/').into()
    }

    /// Podman goes through the compatible API client: bollard rejects some of its states.
    fn podman_api(&self) -> Option<&CompatApi> {
        match self.engine {
            ContainerEngine::Podman => self.compat.as_ref(),
            ContainerEngine::Docker => None,
        }
    }

    pub(crate) async fn pull_image(
        &self,
        image: &str,
        credentials: Option<DockerCredentials>,
    ) -> Result<(), String> {
        if credentials.is_some()
            && let Some(compat) = &self.compat
        {
            info!(image = image, "Pulling image");
            return compat.pull_image(image, credentials.as_ref()).await;
        }
        self.docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: Some(image.to_string()),
                    ..Default::default()
                }),
                None,
                credentials,
            )
            .try_for_each(|info| {
                info!(
                    "{} {:?} {:?} pulling...",
                    image,
                    info.status.as_deref(),
                    info.progress_detail.as_ref()
                );
                future::ok(())
            })
            .await
            .map_err(|err| err.to_string())
    }
}

fn container_from_inspect(inspect: compat::ContainerInspect) -> OrchestratorContainer {
    let state = inspect.state.unwrap_or_default();
    let config = inspect.config.unwrap_or_default();
    OrchestratorContainer {
        id: inspect.id,
        name: DockerOrchestrator::normalize_name(inspect.name),
        state: compat::normalize_state(state.status.as_deref().unwrap_or_default()),
        envs: DockerOrchestrator::convert_labels(config.env.unwrap_or_default()),
        labels: config.labels.unwrap_or_default(),
        restart_count: inspect.restart_count.unwrap_or(0).max(0) as u32,
        started_at: state.started_at,
    }
}

fn container_from_summary(summary: compat::ContainerSummary) -> OrchestratorContainer {
    OrchestratorContainer {
        id: summary.id,
        name: DockerOrchestrator::normalize_name(
            summary.names.and_then(|names| names.into_iter().next()),
        ),
        state: compat::normalize_state(summary.state.as_deref().unwrap_or_default()),
        envs: HashMap::new(),
        labels: summary.labels.unwrap_or_default(),
        restart_count: 0, // Not available in list, will be updated by get()
        started_at: None, // Not available in list, will be updated by get()
    }
}

/// Build the host configuration of a connector container from the daemon options.
pub(crate) fn build_host_config(
    options: Option<&DockerOptions>,
    engine: ContainerEngine,
    proxy_ca_host_path: Option<String>,
) -> HostConfig {
    let mut host_config = HostConfig::default();

    if let Some(docker_opts) = options {
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
            let ulimits_vec: Vec<bollard::models::ResourcesUlimits> = ulimits
                .iter()
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

    if let Some(proxy_ca_host_path) = proxy_ca_host_path {
        // On SELinux hosts (RHEL, SUSE), Podman must relabel the file for the container to read it.
        let mount_options = match engine {
            ContainerEngine::Podman => "ro,z",
            ContainerEngine::Docker => "ro",
        };
        let mut binds = host_config.binds.unwrap_or_default();
        binds.push(format!(
            "{}:{}:{}",
            proxy_ca_host_path, PROXY_CA_CERT_MOUNT_PATH, mount_options
        ));
        host_config.binds = Some(binds);
    }

    host_config
}

#[async_trait]
impl Orchestrator for DockerOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let container_name = connector.container_name();
        if let Some(compat) = self.podman_api() {
            return match compat.inspect_container(&container_name).await {
                Ok(Some(inspect)) => Some(container_from_inspect(inspect)),
                Ok(None) => {
                    debug!(name = container_name, "Could not find container");
                    None
                }
                Err(err) => {
                    error!(name = container_name, error = err, "Could not inspect container");
                    None
                }
            };
        }
        let opts = Some(InspectContainerOptions::default());
        let container = self
            .docker
            .inspect_container(container_name.as_str(), opts)
            .await;
        match container {
            Ok(docker_container) => {
                let state = docker_container.state.unwrap_or_default();
                let config = docker_container.config.unwrap_or_default();
                Some(OrchestratorContainer {
                    id: docker_container.id.unwrap_or_default(),
                    name: DockerOrchestrator::normalize_name(docker_container.name),
                    state: state.status.map(|status| status.to_string()).unwrap_or_default(),
                    envs: DockerOrchestrator::convert_labels(config.env.unwrap_or_default()),
                    labels: config.labels.unwrap_or_default(),
                    restart_count: docker_container.restart_count.unwrap_or(0) as u32,
                    started_at: state.started_at,
                })
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                debug!(name = container_name, "Could not find docker container");
                None
            }
            Err(err) => {
                error!(
                    name = container_name,
                    error = err.to_string(),
                    "Could not inspect docker container"
                );
                None
            }
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let manager_label = format!("opencti-manager={}", settings.manager.id.clone());
        if let Some(compat) = self.podman_api() {
            return match compat.list_containers(&manager_label).await {
                Ok(containers) => containers.into_iter().map(container_from_summary).collect(),
                Err(err) => {
                    error!(error = err, "Error fetching containers");
                    Vec::new()
                }
            };
        }
        let list_container_filters: HashMap<String, Vec<String>> =
            HashMap::from([("label".to_string(), Vec::from([manager_label]))]);

        let container_result = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(list_container_filters),
                ..Default::default()
            }))
            .await;
        match container_result {
            Ok(containers) => containers
                .into_iter()
                .map(|docker_container| {
                    let container_name: Option<String> = docker_container
                        .names
                        .and_then(|names| names.into_iter().next());
                    OrchestratorContainer {
                        id: docker_container.id.unwrap_or_default(),
                        name: DockerOrchestrator::normalize_name(container_name),
                        state: docker_container
                            .state
                            .map(|state| state.to_string())
                            .unwrap_or_default(),
                        envs: HashMap::new(),
                        labels: docker_container.labels.unwrap_or_default(),
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
            .start_container(container_name.as_str(), None::<StartContainerOptions>)
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
        let resolver = Image::new(self.registry.clone());
        let auth = resolver.get_credentials();
        let image = resolver.build_name(connector.image.clone());

        if let Err(err) = self.pull_image(&image, auth).await {
            error!(image = image, error = err, "Error fetching container image");
            return None;
        }

        // Create the container
        let container_env_variables = connector
            .container_envs()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect::<Vec<String>>();
        let labels = self.labels(connector);
        let host_config = build_host_config(
            self.options.as_ref(),
            self.engine,
            ensure_proxy_ca_file(connector),
        );

        let config = ContainerCreateBody {
            image: Some(image),
            env: Some(container_env_variables),
            labels: Some(labels),
            host_config: Some(host_config),
            ..Default::default()
        };

        let create_response = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(connector.container_name()),
                    ..Default::default()
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
        if is_starting
            && let Some(container) = &created
        {
            self.start(container, connector).await;
        }
        // Return the created container
        created
    }

    async fn logs(
        &self,
        _container: &OrchestratorContainer,
        connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        let opts = Some(LogsOptions {
            follow: false,
            stdout: true,
            stderr: true,
            tail: "100".to_string(),
            ..Default::default()
        });
        let logs = self.docker.logs(connector.container_name().as_str(), opts);
        let mut logs_content = Vec::new();
        let result = logs
            .try_for_each(|log| {
                logs_content.push(log.to_string());
                future::ok(())
            })
            .await;
        match result {
            Ok(_) => Some(logs_content),
            Err(err) => {
                error!(
                    name = connector.container_name(),
                    error = err.to_string(),
                    "Could not fetch container logs"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_entries_keep_values_containing_equal_signs() {
        let envs = DockerOrchestrator::convert_labels(vec![
            "OPENCTI_CONFIG_HASH=abc".to_string(),
            "TOKEN=dGVzdA==".to_string(),
            "EMPTY=".to_string(),
            "MALFORMED".to_string(),
        ]);
        assert_eq!(envs.get("OPENCTI_CONFIG_HASH").map(String::as_str), Some("abc"));
        assert_eq!(envs.get("TOKEN").map(String::as_str), Some("dGVzdA=="));
        assert_eq!(envs.get("EMPTY").map(String::as_str), Some(""));
        assert!(!envs.contains_key("MALFORMED"));
    }

    #[test]
    fn container_names_are_normalized_without_panicking() {
        assert_eq!(DockerOrchestrator::normalize_name(Some("/connector".into())), "connector");
        assert_eq!(DockerOrchestrator::normalize_name(Some("connector".into())), "connector");
        assert_eq!(DockerOrchestrator::normalize_name(None), "");
    }

    #[test]
    fn podman_inspect_with_internal_stopped_state_maps_to_a_stopped_connector() {
        let inspect: compat::ContainerInspect = serde_json::from_str(
            r#"{"Id":"abc","Name":"/connector-a","RestartCount":4,
                "State":{"Status":"stopped","StartedAt":"2026-10-21T10:00:00Z"},
                "Config":{"Env":["OPENCTI_CONFIG_HASH=h1","container=podman"],
                          "Labels":{"opencti-connector-id":"A","opencti-manager":"m"}}}"#,
        )
        .unwrap();

        let container = container_from_inspect(inspect);

        assert_eq!(container.id, "abc");
        assert_eq!(container.name, "connector-a");
        assert_eq!(container.state, "exited");
        assert_eq!(container.restart_count, 4);
        assert_eq!(container.started_at.as_deref(), Some("2026-10-21T10:00:00Z"));
        assert_eq!(container.extract_opencti_hash(), "h1");
        assert_eq!(container.extract_opencti_id(), "A");
    }

    #[test]
    fn podman_inspect_without_labels_is_an_unmanaged_container() {
        let inspect: compat::ContainerInspect =
            serde_json::from_str(r#"{"Id":"abc","Name":"/manual","State":null,"Config":{"Labels":null}}"#)
                .unwrap();
        let container = container_from_inspect(inspect);
        assert!(!container.is_managed());
        assert_eq!(container.state, "");
        assert_eq!(container.restart_count, 0);
    }

    #[test]
    fn podman_summary_maps_name_state_and_labels() {
        let summary: compat::ContainerSummary = serde_json::from_str(
            r#"{"Id":"def","Names":["/connector-b"],"State":"stopped","Labels":{"opencti-connector-id":"B"}}"#,
        )
        .unwrap();
        let container = container_from_summary(summary);
        assert_eq!(container.name, "connector-b");
        assert_eq!(container.state, "exited");
        assert_eq!(container.extract_opencti_id(), "B");
    }

    fn proxy_ca_bind(engine: ContainerEngine) -> Vec<String> {
        build_host_config(None, engine, Some("/tmp/ca.crt".to_string()))
            .binds
            .expect("proxy CA bind")
    }

    #[test]
    fn proxy_ca_bind_is_relabeled_for_selinux_with_podman_only() {
        assert_eq!(
            proxy_ca_bind(ContainerEngine::Podman),
            vec![format!("/tmp/ca.crt:{}:ro,z", PROXY_CA_CERT_MOUNT_PATH)]
        );
        assert_eq!(
            proxy_ca_bind(ContainerEngine::Docker),
            vec![format!("/tmp/ca.crt:{}:ro", PROXY_CA_CERT_MOUNT_PATH)]
        );
    }

    #[test]
    fn host_config_applies_daemon_options() {
        let options = DockerOptions {
            network_mode: Some("bridge".into()),
            privileged: Some(false),
            cap_drop: Some(vec!["ALL".into()]),
            shm_size: Some(64),
            ulimits: Some(vec![
                HashMap::from([
                    ("name".to_string(), serde_json::json!("nofile")),
                    ("soft".to_string(), serde_json::json!(1024)),
                    ("hard".to_string(), serde_json::json!(2048)),
                ]),
                HashMap::from([("name".to_string(), serde_json::json!("incomplete"))]),
            ]),
            ..Default::default()
        };

        let host_config = build_host_config(Some(&options), ContainerEngine::Podman, None);

        assert_eq!(host_config.network_mode.as_deref(), Some("bridge"));
        assert_eq!(host_config.privileged, Some(false));
        assert_eq!(host_config.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(host_config.shm_size, Some(64));
        let ulimits = host_config.ulimits.expect("ulimits");
        assert_eq!(ulimits.len(), 1, "incomplete ulimits are skipped");
        assert_eq!(ulimits[0].name.as_deref(), Some("nofile"));
        assert!(host_config.binds.is_none());
        assert!(build_host_config(None, ContainerEngine::Docker, None).network_mode.is_none());
    }
}
