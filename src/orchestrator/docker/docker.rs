use crate::api::{ApiConnector, ConnectorStatus};
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::image::Image;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::HostConfig;
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

    fn build_host_config() -> HostConfig {
        let settings = &crate::config::settings::SETTINGS;
        let Some(docker_opts) = &settings.opencti.daemon.docker else {
            return HostConfig::default();
        };

        let ulimits = docker_opts.ulimits.as_ref().and_then(|ulimits| {
            let vec: Vec<bollard::models::ResourcesUlimits> = ulimits
                .iter()
                .filter_map(|ulimit_map| {
                    Some(bollard::models::ResourcesUlimits {
                        name: ulimit_map.get("name")?.as_str()?.to_string().into(),
                        soft: ulimit_map.get("soft")?.as_i64(),
                        hard: ulimit_map.get("hard")?.as_i64(),
                    })
                })
                .collect();
            if vec.is_empty() { None } else { Some(vec) }
        });

        HostConfig {
            network_mode: docker_opts.network_mode.clone(),
            extra_hosts: docker_opts.extra_hosts.clone(),
            dns: docker_opts.dns.clone(),
            dns_search: docker_opts.dns_search.clone(),
            privileged: docker_opts.privileged,
            cap_add: docker_opts.cap_add.clone(),
            cap_drop: docker_opts.cap_drop.clone(),
            security_opt: docker_opts.security_opt.clone(),
            userns_mode: docker_opts.userns_mode.clone(),
            pid_mode: docker_opts.pid_mode.clone(),
            ipc_mode: docker_opts.ipc_mode.clone(),
            uts_mode: docker_opts.uts_mode.clone(),
            runtime: docker_opts.runtime.clone(),
            shm_size: docker_opts.shm_size,
            sysctls: docker_opts.sysctls.clone(),
            ulimits,
            ..Default::default()
        }
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
            }
            Err(_) => {
                debug!(name = container_name, "Could not find docker container");
                None
            }
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = &crate::config::settings::SETTINGS;
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
        let settings = &crate::config::settings::SETTINGS;
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = Image::new(registry_config);
        let auth = resolver.get_credentials();
        let image = resolver.build_name(connector.image.clone());

        let deploy_response = self
            .docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: image.as_str(),
                    ..Default::default()
                }),
                None,
                auth,
            )
            .try_for_each(|info| {
                info!(
                    "{} {:?} {:?} pulling...",
                    image,
                    info.status.as_deref(),
                    info.progress.as_deref()
                );
                future::ok(())
            })
            .await;

        if let Err(e) = deploy_response {
            error!(
                image = image,
                error = e.to_string(),
                "Error fetching container image"
            );
            return None;
        }

        // Create the container
        let container_env_variables = connector
            .container_envs()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect::<Vec<String>>();
        let labels = self.labels(connector);

        // Build host config with Docker options
        let host_config = DockerOrchestrator::build_host_config();

        let config = Config {
            image: Some(image),
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
                    ..Default::default()
                }),
                config,
            )
            .await;

        if let Err(err) = create_response {
            error!(error = err.to_string(), "Error creating container");
        }

        // Get the created connector
        let created = self.get(connector).await;
        // Start the container if needed
        let is_starting = connector.requested_status.clone().eq("starting");
        if is_starting {
            if let Some(container) = &created {
                self.start(container, connector).await;
            }
        }
        // Return the created container
        created
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
