use crate::api::{ApiConnector, ConnectorStatus};
use crate::orchestrator::image::Image;
use crate::orchestrator::swarm::SwarmOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::auth::DockerCredentials;
use bollard::models::{
    Limit, NetworkAttachmentConfig, ResourceObject, ResourcesUlimits, ServiceSpec,
    ServiceSpecMode, ServiceSpecModeReplicated, TaskSpec, TaskSpecContainerSpec,
    TaskSpecContainerSpecDnsConfig, TaskSpecPlacement, TaskSpecPlacementPreferences,
    TaskSpecPlacementSpread, TaskSpecResources, TaskSpecRestartPolicy,
    TaskSpecRestartPolicyConditionEnum,
};
use bollard::query_parameters::{
    CreateImageOptions, InspectServiceOptions, ListServicesOptions, ListTasksOptions, LogsOptions,
    UpdateServiceOptions,
};
use bollard::Docker;
use futures::future;
use futures::TryStreamExt;
use std::collections::HashMap;
use tracing::{debug, error, info};

impl SwarmOrchestrator {
    pub fn new(config: crate::config::settings::Swarm) -> Self {
        let docker = Docker::connect_with_socket_defaults().unwrap();
        Self { docker, config }
    }

    async fn get_task_info(&self, service_name: &str) -> (u32, Option<String>, String) {
        let filters = HashMap::from([(
            "service".to_string(),
            vec![service_name.to_string()],
        )]);
        let task_options = Some(ListTasksOptions {
            filters: Some(filters),
            ..Default::default()
        });
        match self.docker.list_tasks(task_options).await {
            Ok(tasks) => {
                let total_tasks = tasks.len();

                // Find the most recent running task
                let running_task = tasks.iter().find(|t| {
                    t.status
                        .as_ref()
                        .and_then(|s| s.state.as_ref())
                        .map(|s| {
                            let state_str = format!("{:?}", s).to_lowercase();
                            state_str == "running" || state_str.contains("running")
                        })
                        .unwrap_or(false)
                });

                match running_task {
                    Some(task) => {
                        let started_at =
                            task.status.as_ref().and_then(|s| s.timestamp.clone());
                        let restart_count =
                            if total_tasks > 1 { (total_tasks - 1) as u32 } else { 0 };
                        (restart_count, started_at, "running".to_string())
                    }
                    None => {
                        let restart_count = total_tasks as u32;
                        (restart_count, None, "stopped".to_string())
                    }
                }
            }
            Err(_) => (0, None, "unknown".to_string()),
        }
    }
}

#[async_trait]
impl Orchestrator for SwarmOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let service_name = connector.container_name();
        let service = self
            .docker
            .inspect_service(&service_name, None::<InspectServiceOptions>)
            .await;
        match service {
            Ok(svc) => {
                let spec = svc.spec.clone().unwrap_or_default();
                let labels = spec.labels.unwrap_or_default();

                let envs = spec
                    .task_template
                    .and_then(|t| t.container_spec)
                    .and_then(|c| c.env)
                    .map(|env_list| {
                        env_list
                            .iter()
                            .filter_map(|env| {
                                let parts: Vec<&str> = env.splitn(2, '=').collect();
                                if parts.len() == 2 {
                                    Some((parts[0].to_string(), parts[1].to_string()))
                                } else {
                                    None
                                }
                            })
                            .collect::<HashMap<String, String>>()
                    })
                    .unwrap_or_default();

                let (restart_count, started_at, state) =
                    self.get_task_info(&service_name).await;

                Some(OrchestratorContainer {
                    id: svc.id.unwrap_or_default(),
                    name: service_name,
                    state,
                    labels,
                    envs,
                    restart_count,
                    started_at,
                })
            }
            Err(_) => {
                debug!(name = service_name, "Could not find swarm service");
                None
            }
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let manager_label = format!("opencti-manager={}", settings.manager.id);
        let filters: HashMap<String, Vec<String>> =
            HashMap::from([("label".to_string(), vec![manager_label])]);

        let options = Some(ListServicesOptions {
            filters: Some(filters),
            ..Default::default()
        });
        match self.docker.list_services(options).await {
            Ok(services) => services
                .into_iter()
                .filter_map(|svc| {
                    let spec = svc.spec?;
                    let name = spec.name.clone()?;
                    let labels = spec.labels.unwrap_or_default();
                    Some(OrchestratorContainer {
                        id: svc.id.unwrap_or_default(),
                        name,
                        state: "unknown".to_string(),
                        envs: HashMap::new(),
                        labels,
                        restart_count: 0,
                        started_at: None,
                    })
                })
                .collect(),
            Err(err) => {
                error!(error = err.to_string(), "Error fetching swarm services");
                Vec::new()
            }
        }
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        let service_name = connector.container_name();
        if let Ok(svc) = self
            .docker
            .inspect_service(&service_name, None::<InspectServiceOptions>)
            .await
        {
            let version = svc.version.as_ref().and_then(|v| v.index).unwrap_or(0) as i32;
            let mut spec = svc.spec.unwrap_or_default();

            if let Some(ref mut mode) = spec.mode {
                if let Some(ref mut replicated) = mode.replicated {
                    replicated.replicas = Some(1);
                }
            } else {
                spec.mode = Some(ServiceSpecMode {
                    replicated: Some(ServiceSpecModeReplicated {
                        replicas: Some(1),
                    }),
                    ..Default::default()
                });
            }

            let options = UpdateServiceOptions {
                version,
                ..Default::default()
            };
            let _ = self
                .docker
                .update_service(&service_name, spec, options, None::<DockerCredentials>)
                .await;
        }
    }

    async fn stop(&self, _container: &OrchestratorContainer, _connector: &ApiConnector) -> () {
        let service_name = _connector.container_name();
        if let Ok(svc) = self
            .docker
            .inspect_service(&service_name, None::<InspectServiceOptions>)
            .await
        {
            let version = svc.version.as_ref().and_then(|v| v.index).unwrap_or(0) as i32;
            let mut spec = svc.spec.unwrap_or_default();

            if let Some(ref mut mode) = spec.mode {
                if let Some(ref mut replicated) = mode.replicated {
                    replicated.replicas = Some(0);
                }
            } else {
                spec.mode = Some(ServiceSpecMode {
                    replicated: Some(ServiceSpecModeReplicated {
                        replicas: Some(0),
                    }),
                    ..Default::default()
                });
            }

            let options = UpdateServiceOptions {
                version,
                ..Default::default()
            };
            let _ = self
                .docker
                .update_service(&service_name, spec, options, None::<DockerCredentials>)
                .await;
        }
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        let service_name = container.name.as_str();
        match self.docker.delete_service(service_name).await {
            Ok(_) => {
                info!(name = service_name, "Removed swarm service");
            }
            Err(err) => {
                error!(
                    name = service_name,
                    error = err.to_string(),
                    "Could not remove swarm service"
                );
            }
        }
    }

    async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let container = self.get(connector).await;
        if container.is_some() {
            let _ = self.remove(&container.unwrap()).await;
        }
        self.deploy(connector).await
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let settings = crate::settings();
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = Image::new(registry_config);
        let auth = resolver.get_credentials();
        let image = resolver.build_name(connector.image.clone());

        let pull_result = self
            .docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: Some(image.clone()),
                    ..Default::default()
                }),
                None,
                auth.clone(),
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

        match pull_result {
            Ok(_) => {
                let container_env_variables: Vec<String> = connector
                    .container_envs()
                    .into_iter()
                    .map(|config| format!("{}={}", config.key, config.value))
                    .collect();
                let labels = self.labels(connector);
                let swarm_opts = &self.config;

                // Build container spec with all swarm options
                let mut container_spec = TaskSpecContainerSpec {
                    image: Some(image.clone()),
                    env: Some(container_env_variables),
                    ..Default::default()
                };

                if let Some(extra_hosts) = &swarm_opts.extra_hosts {
                    container_spec.hosts = Some(extra_hosts.clone());
                }
                if swarm_opts.dns.is_some() || swarm_opts.dns_search.is_some() {
                    container_spec.dns_config = Some(TaskSpecContainerSpecDnsConfig {
                        nameservers: swarm_opts.dns.clone(),
                        search: swarm_opts.dns_search.clone(),
                        options: None,
                    });
                }
                if let Some(cap_add) = &swarm_opts.cap_add {
                    container_spec.capability_add = Some(cap_add.clone());
                }
                if let Some(cap_drop) = &swarm_opts.cap_drop {
                    container_spec.capability_drop = Some(cap_drop.clone());
                }
                if let Some(sysctls) = &swarm_opts.sysctls {
                    container_spec.sysctls = Some(sysctls.clone());
                }
                if let Some(ulimits) = &swarm_opts.ulimits {
                    let ulimits_vec: Vec<ResourcesUlimits> = ulimits
                        .iter()
                        .filter_map(|ulimit_map| {
                            if let (Some(name), Some(soft), Some(hard)) = (
                                ulimit_map.get("name").and_then(|v| v.as_str()),
                                ulimit_map.get("soft").and_then(|v| v.as_i64()),
                                ulimit_map.get("hard").and_then(|v| v.as_i64()),
                            ) {
                                Some(ResourcesUlimits {
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
                        container_spec.ulimits = Some(ulimits_vec);
                    }
                }
                if let Some(hostname) = &swarm_opts.hostname {
                    container_spec.hostname = Some(hostname.clone());
                }
                if let Some(user) = &swarm_opts.user {
                    container_spec.user = Some(user.clone());
                }
                if let Some(read_only) = swarm_opts.read_only {
                    container_spec.read_only = Some(read_only);
                }
                if let Some(init) = swarm_opts.init {
                    container_spec.init = Some(init);
                }
                if let Some(stop_grace_period) = swarm_opts.stop_grace_period {
                    container_spec.stop_grace_period = Some(stop_grace_period);
                }

                // Build network attachments
                let networks = swarm_opts.network.as_ref().map(|net| {
                    vec![NetworkAttachmentConfig {
                        target: Some(net.clone()),
                        ..Default::default()
                    }]
                });

                // Build resource limits and reservations
                let resources = swarm_opts.resources.as_ref().map(|res| {
                    let limits = if res.cpu_limit.is_some() || res.memory_limit.is_some() {
                        Some(Limit {
                            nano_cpus: res.cpu_limit,
                            memory_bytes: res.memory_limit,
                            ..Default::default()
                        })
                    } else {
                        None
                    };
                    let reservations =
                        if res.cpu_reservation.is_some() || res.memory_reservation.is_some() {
                            Some(ResourceObject {
                                nano_cpus: res.cpu_reservation,
                                memory_bytes: res.memory_reservation,
                                ..Default::default()
                            })
                        } else {
                            None
                        };
                    TaskSpecResources {
                        limits,
                        reservations,
                    }
                });

                // Build placement constraints and preferences
                let placement = if swarm_opts.placement_constraints.is_some()
                    || swarm_opts.placement_preferences.is_some()
                {
                    let preferences =
                        swarm_opts
                            .placement_preferences
                            .as_ref()
                            .map(|prefs| {
                                prefs
                                    .iter()
                                    .map(|p| TaskSpecPlacementPreferences {
                                        spread: Some(TaskSpecPlacementSpread {
                                            spread_descriptor: Some(p.clone()),
                                        }),
                                    })
                                    .collect()
                            });
                    Some(TaskSpecPlacement {
                        constraints: swarm_opts.placement_constraints.clone(),
                        preferences,
                        ..Default::default()
                    })
                } else {
                    None
                };

                // Build restart policy
                let restart_policy = if swarm_opts.restart_condition.is_some()
                    || swarm_opts.restart_delay.is_some()
                    || swarm_opts.restart_max_attempts.is_some()
                {
                    let condition =
                        swarm_opts
                            .restart_condition
                            .as_ref()
                            .map(|c| match c.as_str() {
                                "none" => TaskSpecRestartPolicyConditionEnum::NONE,
                                "on-failure" => {
                                    TaskSpecRestartPolicyConditionEnum::ON_FAILURE
                                }
                                _ => TaskSpecRestartPolicyConditionEnum::ANY,
                            });
                    Some(TaskSpecRestartPolicy {
                        condition,
                        delay: swarm_opts.restart_delay,
                        max_attempts: swarm_opts.restart_max_attempts,
                        ..Default::default()
                    })
                } else {
                    None
                };

                let is_starting = connector.requested_status.clone().eq("starting");
                let replicas = if is_starting { 1 } else { 0 };

                let service_spec = ServiceSpec {
                    name: Some(connector.container_name()),
                    labels: Some(labels),
                    task_template: Some(TaskSpec {
                        container_spec: Some(container_spec),
                        networks,
                        resources,
                        placement,
                        restart_policy,
                        ..Default::default()
                    }),
                    mode: Some(ServiceSpecMode {
                        replicated: Some(ServiceSpecModeReplicated {
                            replicas: Some(replicas),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                match self.docker.create_service(service_spec, auth).await {
                    Ok(response) => {
                        debug!(id = ?response.id, "Swarm service created");
                    }
                    Err(err) => {
                        error!(
                            error = err.to_string(),
                            "Error creating swarm service"
                        );
                        return None;
                    }
                }

                self.get(connector).await
            }
            Err(e) => {
                error!(
                    image = image,
                    error = e.to_string(),
                    "Error fetching container image"
                );
                None
            }
        }
    }

    async fn logs(
        &self,
        _container: &OrchestratorContainer,
        connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        let service_name = connector.container_name();

        // Retrieve logs via tasks: find the running task's container and get its logs
        let filters = HashMap::from([(
            "service".to_string(),
            vec![service_name.clone()],
        )]);
        let task_options = Some(ListTasksOptions {
            filters: Some(filters),
            ..Default::default()
        });

        match self.docker.list_tasks(task_options).await {
            Ok(tasks) => {
                // Find a running task with a container ID
                for task in &tasks {
                    let is_running = task
                        .status
                        .as_ref()
                        .and_then(|s| s.state.as_ref())
                        .map(|s| {
                            let state_str = format!("{:?}", s).to_lowercase();
                            state_str == "running" || state_str.contains("running")
                        })
                        .unwrap_or(false);

                    if !is_running {
                        continue;
                    }

                    let container_id = task
                        .status
                        .as_ref()
                        .and_then(|s| s.container_status.as_ref())
                        .and_then(|cs| cs.container_id.as_ref());

                    if let Some(cid) = container_id {
                        let opts = Some(LogsOptions {
                            follow: false,
                            stdout: true,
                            stderr: true,
                            tail: "100".to_string(),
                            ..Default::default()
                        });
                        let logs = self.docker.logs(cid.as_str(), opts);
                        let mut logs_content = Vec::new();
                        match logs
                            .try_for_each(|log| {
                                logs_content.push(log.to_string());
                                future::ok(())
                            })
                            .await
                        {
                            Ok(_) => return Some(logs_content),
                            Err(err) => {
                                debug!(
                                    error = err.to_string(),
                                    "Could not fetch logs from task container, trying next task"
                                );
                                continue;
                            }
                        }
                    }
                }
                None
            }
            Err(err) => {
                error!(
                    error = err.to_string(),
                    "Error fetching tasks for swarm service"
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
