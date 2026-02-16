use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Swarm;
use crate::orchestrator::image::Image;
use crate::orchestrator::swarm::SwarmOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::Docker;
use bollard::models::{
    NetworkAttachmentConfig, ServiceSpec, ServiceSpecMode, ServiceSpecModeReplicated, TaskSpec,
    TaskSpecContainerSpec, TaskSpecContainerSpecDnsConfig, TaskSpecPlacement,
    TaskSpecRestartPolicy, TaskSpecRestartPolicyConditionEnum,
};
use bollard::query_parameters::{
    ListServicesOptionsBuilder, ListTasksOptionsBuilder, LogsOptions, UpdateServiceOptionsBuilder,
};
use futures::TryStreamExt;
use futures::future;
use std::collections::HashMap;
use tracing::{debug, error, info};

impl SwarmOrchestrator {
    pub fn new(config: Swarm) -> Self {
        let docker = Docker::connect_with_socket_defaults().unwrap();
        Self { docker, config }
    }

    fn build_service_spec(&self, connector: &ApiConnector, labels: HashMap<String, String>) -> ServiceSpec {
        let image_name = {
            let settings = crate::settings();
            let registry_config = settings.opencti.daemon.registry.clone();
            let resolver = Image::new(registry_config);
            resolver.build_name(connector.image.clone())
        };

        let container_env_variables: Vec<String> = connector
            .container_envs()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect();

        // Build container spec
        let mut container_spec = TaskSpecContainerSpec {
            image: Some(image_name),
            env: Some(container_env_variables),
            labels: Some(labels.clone()),
            ..Default::default()
        };

        // Apply swarm-specific container options
        if self.config.hosts.is_some() {
            container_spec.hosts = self.config.hosts.clone();
        }
        if self.config.cap_add.is_some() {
            container_spec.capability_add = self.config.cap_add.clone();
        }
        if self.config.cap_drop.is_some() {
            container_spec.capability_drop = self.config.cap_drop.clone();
        }
        if self.config.sysctls.is_some() {
            container_spec.sysctls = self.config.sysctls.clone();
        }
        if self.config.dns.is_some() || self.config.dns_search.is_some() {
            container_spec.dns_config = Some(TaskSpecContainerSpecDnsConfig {
                nameservers: self.config.dns.clone(),
                search: self.config.dns_search.clone(),
                options: None,
            });
        }

        // Build placement
        let placement = self.config.placement_constraints.as_ref().map(|constraints| {
            TaskSpecPlacement {
                constraints: Some(constraints.clone()),
                ..Default::default()
            }
        });

        // Build restart policy
        let restart_policy = Some(TaskSpecRestartPolicy {
            condition: Some(
                self.config
                    .restart_condition
                    .as_deref()
                    .map(|c| match c {
                        "none" => TaskSpecRestartPolicyConditionEnum::NONE,
                        "on-failure" => TaskSpecRestartPolicyConditionEnum::ON_FAILURE,
                        "any" => TaskSpecRestartPolicyConditionEnum::ANY,
                        _ => TaskSpecRestartPolicyConditionEnum::ANY,
                    })
                    .unwrap_or(TaskSpecRestartPolicyConditionEnum::ANY),
            ),
            max_attempts: self.config.restart_max_attempts,
            ..Default::default()
        });

        // Build task spec
        let task_template = TaskSpec {
            container_spec: Some(container_spec),
            placement,
            restart_policy,
            ..Default::default()
        };

        // Build networks
        let networks = self.config.network.as_ref().map(|net| {
            vec![NetworkAttachmentConfig {
                target: Some(net.clone()),
                ..Default::default()
            }]
        });

        // Build update config
        let update_config = if self.config.update_parallelism.is_some() || self.config.update_delay.is_some() {
            Some(bollard::models::ServiceSpecUpdateConfig {
                parallelism: self.config.update_parallelism,
                delay: self.config.update_delay,
                ..Default::default()
            })
        } else {
            None
        };

        ServiceSpec {
            name: Some(connector.container_name()),
            labels: Some(labels),
            task_template: Some(task_template),
            mode: Some(ServiceSpecMode {
                replicated: Some(ServiceSpecModeReplicated {
                    replicas: Some(1),
                }),
                ..Default::default()
            }),
            networks,
            update_config,
            ..Default::default()
        }
    }

    async fn get_service_version(&self, service_name: &str) -> Option<u64> {
        match self.docker.inspect_service(service_name, None::<bollard::query_parameters::InspectServiceOptions>).await {
            Ok(service) => service.version.and_then(|v| v.index),
            Err(_) => None,
        }
    }

    async fn get_running_task_for_service(&self, service_name: &str) -> Option<bollard::models::Task> {
        let filters: HashMap<String, Vec<String>> = HashMap::from([
            ("service".to_string(), vec![service_name.to_string()]),
            ("desired-state".to_string(), vec!["running".to_string()]),
        ]);
        let opts = ListTasksOptionsBuilder::default()
            .filters(&filters)
            .build();
        match self.docker.list_tasks(Some(opts)).await {
            Ok(tasks) => tasks.into_iter().next(),
            Err(_) => None,
        }
    }

    fn service_to_container(&self, service: &bollard::models::Service, task_state: Option<&str>) -> OrchestratorContainer {
        let spec = service.spec.as_ref();
        let service_id = service.id.clone().unwrap_or_default();
        let service_name = spec.and_then(|s| s.name.clone()).unwrap_or_default();
        let labels = spec
            .and_then(|s| s.labels.clone())
            .unwrap_or_default();
        let envs = spec
            .and_then(|s| s.task_template.as_ref())
            .and_then(|t| t.container_spec.as_ref())
            .and_then(|c| c.env.as_ref())
            .map(|env_list| Self::convert_env_to_map(env_list))
            .unwrap_or_default();
        let state = task_state.unwrap_or("shutdown").to_string();

        OrchestratorContainer {
            id: service_id,
            name: service_name,
            state,
            labels,
            envs,
            restart_count: 0,
            started_at: None,
        }
    }

    fn convert_env_to_map(envs: &[String]) -> HashMap<String, String> {
        envs.iter()
            .filter_map(|env| {
                let mut parts = env.splitn(2, '=');
                match (parts.next(), parts.next()) {
                    (Some(key), Some(value)) => Some((key.to_string(), value.to_string())),
                    _ => None,
                }
            })
            .collect()
    }
}

#[async_trait]
impl Orchestrator for SwarmOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let service_name = connector.container_name();
        let service = self
            .docker
            .inspect_service(service_name.as_str(), None::<bollard::query_parameters::InspectServiceOptions>)
            .await;
        match service {
            Ok(svc) => {
                // Get the most recent task to determine state
                let task = self.get_running_task_for_service(service_name.as_str()).await;
                let task_state = task
                    .as_ref()
                    .and_then(|t| t.status.as_ref())
                    .and_then(|s| s.state.as_ref())
                    .map(|s| s.to_string());
                let started_at = task
                    .as_ref()
                    .and_then(|t| t.status.as_ref())
                    .and_then(|s| s.timestamp.clone());
                let restart_count = self.count_restarts_for_service(service_name.as_str()).await;

                let mut container = self.service_to_container(&svc, task_state.as_deref());
                container.started_at = started_at;
                container.restart_count = restart_count;
                Some(container)
            }
            Err(_) => {
                debug!(name = service_name, "Could not find swarm service");
                None
            }
        }
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let manager_label = format!("opencti-manager={}", settings.manager.id.clone());
        let filters: HashMap<String, Vec<String>> =
            HashMap::from([("label".to_string(), vec![manager_label])]);
        let opts = ListServicesOptionsBuilder::default()
            .filters(&filters)
            .status(true)
            .build();
        match self.docker.list_services(Some(opts)).await {
            Ok(services) => {
                services
                    .iter()
                    .map(|svc| {
                        // For list, we derive state from service_status
                        let running = svc
                            .service_status
                            .as_ref()
                            .and_then(|s| s.running_tasks)
                            .unwrap_or(0);
                        let state = if running > 0 { "running" } else { "shutdown" };
                        self.service_to_container(svc, Some(state))
                    })
                    .collect()
            }
            Err(err) => {
                error!(error = err.to_string(), "Error fetching swarm services");
                Vec::new()
            }
        }
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        let service_name = connector.container_name();
        let version = self.get_service_version(service_name.as_str()).await;
        match version {
            Some(v) => {
                // Get current spec and set replicas to 1
                if let Ok(svc) = self.docker.inspect_service(service_name.as_str(), None::<bollard::query_parameters::InspectServiceOptions>).await {
                    let mut spec = svc.spec.unwrap_or_default();
                    spec.mode = Some(ServiceSpecMode {
                        replicated: Some(ServiceSpecModeReplicated { replicas: Some(1) }),
                        ..Default::default()
                    });
                    let opts = UpdateServiceOptionsBuilder::default()
                        .version(v as i32)
                        .build();
                    let _ = self.docker.update_service(service_name.as_str(), spec, opts, None).await;
                }
            }
            None => {
                error!(name = service_name, "Could not get service version for start");
            }
        }
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        let service_name = connector.container_name();
        let version = self.get_service_version(service_name.as_str()).await;
        match version {
            Some(v) => {
                if let Ok(svc) = self.docker.inspect_service(service_name.as_str(), None::<bollard::query_parameters::InspectServiceOptions>).await {
                    let mut spec = svc.spec.unwrap_or_default();
                    spec.mode = Some(ServiceSpecMode {
                        replicated: Some(ServiceSpecModeReplicated { replicas: Some(0) }),
                        ..Default::default()
                    });
                    let opts = UpdateServiceOptionsBuilder::default()
                        .version(v as i32)
                        .build();
                    let _ = self.docker.update_service(service_name.as_str(), spec, opts, None).await;
                }
            }
            None => {
                error!(name = service_name, "Could not get service version for stop");
            }
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
        let service_name = connector.container_name();
        let settings = crate::settings();
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = Image::new(registry_config);
        let auth = resolver.get_credentials();

        let version = self.get_service_version(service_name.as_str()).await;
        match version {
            Some(v) => {
                let labels = self.labels(connector);
                let spec = self.build_service_spec(connector, labels);
                let opts = UpdateServiceOptionsBuilder::default()
                    .version(v as i32)
                    .build();
                match self.docker.update_service(service_name.as_str(), spec, opts, auth).await {
                    Ok(_) => {
                        info!(name = service_name, "Refreshed swarm service");
                        self.get(connector).await
                    }
                    Err(err) => {
                        error!(
                            name = service_name,
                            error = err.to_string(),
                            "Error refreshing swarm service"
                        );
                        None
                    }
                }
            }
            None => {
                // Service doesn't exist, deploy fresh
                self.deploy(connector).await
            }
        }
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let settings = crate::settings();
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = Image::new(registry_config);
        let auth = resolver.get_credentials();
        let image = resolver.build_name(connector.image.clone());

        let labels = self.labels(connector);
        let spec = self.build_service_spec(connector, labels);

        // If connector should not be started, set replicas to 0
        let final_spec = if connector.requested_status != "starting" {
            let mut s = spec;
            s.mode = Some(ServiceSpecMode {
                replicated: Some(ServiceSpecModeReplicated { replicas: Some(0) }),
                ..Default::default()
            });
            s
        } else {
            spec
        };

        match self.docker.create_service(final_spec, auth).await {
            Ok(response) => {
                info!(
                    image = image,
                    id = ?response.id,
                    "Created swarm service"
                );
                self.get(connector).await
            }
            Err(err) => {
                error!(
                    image = image,
                    error = err.to_string(),
                    "Error creating swarm service"
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
        // Get the running task for this service, then fetch its container logs
        let task = self.get_running_task_for_service(service_name.as_str()).await;
        let container_id = task
            .as_ref()
            .and_then(|t| t.status.as_ref())
            .and_then(|s| s.container_status.as_ref())
            .and_then(|cs| cs.container_id.clone());

        match container_id {
            Some(cid) => {
                let opts = Some(LogsOptions {
                    follow: false,
                    stdout: true,
                    stderr: true,
                    tail: "100".to_string(),
                    ..Default::default()
                });
                let logs = self.docker.logs(cid.as_str(), opts);
                let mut logs_content = Vec::new();
                logs.try_for_each(|log| {
                    logs_content.push(log.to_string());
                    future::ok(())
                })
                .await
                .unwrap_or(());
                Some(logs_content)
            }
            None => {
                debug!(name = service_name, "No running task container for logs");
                None
            }
        }
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" | "starting" | "ready" => ConnectorStatus::Started,
            _ => ConnectorStatus::Stopped,
        }
    }
}

impl SwarmOrchestrator {
    async fn count_restarts_for_service(&self, service_name: &str) -> u32 {
        // Count failed/shutdown tasks as restarts
        let filters: HashMap<String, Vec<String>> = HashMap::from([
            ("service".to_string(), vec![service_name.to_string()]),
        ]);
        let opts = ListTasksOptionsBuilder::default()
            .filters(&filters)
            .build();
        match self.docker.list_tasks(Some(opts)).await {
            Ok(tasks) => {
                // Count tasks that are not the current running one (i.e. previous attempts)
                let total = tasks.len();
                if total > 1 { (total - 1) as u32 } else { 0 }
            }
            Err(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::OrchestratorContainer;

    fn make_config() -> Swarm {
        Swarm {
            network: Some("my-overlay".to_string()),
            placement_constraints: Some(vec!["node.role==worker".to_string()]),
            update_parallelism: Some(1),
            update_delay: Some(5_000_000_000),
            restart_condition: Some("any".to_string()),
            restart_max_attempts: Some(3),
            dns: Some(vec!["8.8.8.8".to_string()]),
            dns_search: Some(vec!["example.com".to_string()]),
            hosts: Some(vec!["myhost:10.0.0.1".to_string()]),
            cap_add: Some(vec!["NET_ADMIN".to_string()]),
            cap_drop: Some(vec!["MKNOD".to_string()]),
            sysctls: Some(HashMap::from([("net.core.somaxconn".to_string(), "1024".to_string())])),
        }
    }

    fn make_minimal_config() -> Swarm {
        Swarm {
            network: None,
            placement_constraints: None,
            update_parallelism: None,
            update_delay: None,
            restart_condition: None,
            restart_max_attempts: None,
            dns: None,
            dns_search: None,
            hosts: None,
            cap_add: None,
            cap_drop: None,
            sysctls: None,
        }
    }

    #[test]
    fn test_state_converter_running() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "running".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Started);
    }

    #[test]
    fn test_state_converter_starting() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "starting".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Started);
    }

    #[test]
    fn test_state_converter_ready() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "ready".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Started);
    }

    #[test]
    fn test_state_converter_shutdown() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "shutdown".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Stopped);
    }

    #[test]
    fn test_state_converter_failed() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "failed".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Stopped);
    }

    #[test]
    fn test_state_converter_complete() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let container = OrchestratorContainer {
            id: "svc1".into(),
            name: "test-service".into(),
            state: "complete".into(),
            labels: HashMap::new(),
            envs: HashMap::new(),
            restart_count: 0,
            started_at: None,
        };
        assert_eq!(orchestrator.state_converter(&container), ConnectorStatus::Stopped);
    }

    #[test]
    fn test_convert_env_to_map() {
        let envs = vec![
            "KEY1=value1".to_string(),
            "KEY2=value2".to_string(),
            "KEY3=value=with=equals".to_string(),
            "INVALID".to_string(),
        ];
        let map = SwarmOrchestrator::convert_env_to_map(&envs);
        assert_eq!(map.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(map.get("KEY2"), Some(&"value2".to_string()));
        assert_eq!(map.get("KEY3"), Some(&"value=with=equals".to_string()));
        assert_eq!(map.get("INVALID"), None);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn test_convert_env_to_map_empty() {
        let envs: Vec<String> = vec![];
        let map = SwarmOrchestrator::convert_env_to_map(&envs);
        assert!(map.is_empty());
    }

    #[test]
    fn test_service_to_container() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let service = bollard::models::Service {
            id: Some("svc-123".to_string()),
            spec: Some(ServiceSpec {
                name: Some("my-connector".to_string()),
                labels: Some(HashMap::from([
                    ("opencti-manager".to_string(), "mgr-1".to_string()),
                    ("opencti-connector-id".to_string(), "conn-1".to_string()),
                ])),
                task_template: Some(TaskSpec {
                    container_spec: Some(TaskSpecContainerSpec {
                        env: Some(vec![
                            "OPENCTI_URL=http://localhost:4000".to_string(),
                            "OPENCTI_CONFIG_HASH=abc123".to_string(),
                        ]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container = orchestrator.service_to_container(&service, Some("running"));
        assert_eq!(container.id, "svc-123");
        assert_eq!(container.name, "my-connector");
        assert_eq!(container.state, "running");
        assert_eq!(container.labels.get("opencti-connector-id"), Some(&"conn-1".to_string()));
        assert_eq!(container.envs.get("OPENCTI_CONFIG_HASH"), Some(&"abc123".to_string()));
        assert!(container.is_managed());
        assert_eq!(container.extract_opencti_id(), "conn-1");
    }

    #[test]
    fn test_service_to_container_defaults() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let service = bollard::models::Service {
            id: None,
            spec: None,
            ..Default::default()
        };
        let container = orchestrator.service_to_container(&service, None);
        assert_eq!(container.id, "");
        assert_eq!(container.name, "");
        assert_eq!(container.state, "shutdown");
        assert!(container.labels.is_empty());
        assert!(container.envs.is_empty());
    }

    #[test]
    fn test_build_service_spec_full_config() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_config(),
        };
        let connector = ApiConnector {
            id: "conn-1".to_string(),
            name: "Test Connector".to_string(),
            image: "opencti/connector-test:latest".to_string(),
            contract_hash: "hash123".to_string(),
            current_status: Some("stopped".to_string()),
            requested_status: "starting".to_string(),
            contract_configuration: vec![],
        };
        let labels = HashMap::from([
            ("opencti-manager".to_string(), "mgr-1".to_string()),
            ("opencti-connector-id".to_string(), "conn-1".to_string()),
        ]);

        let spec = orchestrator.build_service_spec(&connector, labels);
        assert_eq!(spec.name, Some("test-connector".to_string()));

        // Check mode
        let mode = spec.mode.unwrap();
        assert_eq!(mode.replicated.unwrap().replicas, Some(1));

        // Check networks
        let networks = spec.networks.unwrap();
        assert_eq!(networks.len(), 1);
        assert_eq!(networks[0].target, Some("my-overlay".to_string()));

        // Check update config
        let update_config = spec.update_config.unwrap();
        assert_eq!(update_config.parallelism, Some(1));
        assert_eq!(update_config.delay, Some(5_000_000_000));

        // Check task template
        let task_template = spec.task_template.unwrap();

        // Check container spec
        let container_spec = task_template.container_spec.unwrap();
        assert!(container_spec.image.is_some());
        assert_eq!(container_spec.hosts, Some(vec!["myhost:10.0.0.1".to_string()]));
        assert_eq!(container_spec.capability_add, Some(vec!["NET_ADMIN".to_string()]));
        assert_eq!(container_spec.capability_drop, Some(vec!["MKNOD".to_string()]));
        assert_eq!(container_spec.sysctls, Some(HashMap::from([("net.core.somaxconn".to_string(), "1024".to_string())])));

        // Check DNS
        let dns_config = container_spec.dns_config.unwrap();
        assert_eq!(dns_config.nameservers, Some(vec!["8.8.8.8".to_string()]));
        assert_eq!(dns_config.search, Some(vec!["example.com".to_string()]));

        // Check placement
        let placement = task_template.placement.unwrap();
        assert_eq!(placement.constraints, Some(vec!["node.role==worker".to_string()]));

        // Check restart policy
        let restart_policy = task_template.restart_policy.unwrap();
        assert_eq!(restart_policy.condition, Some(TaskSpecRestartPolicyConditionEnum::ANY));
        assert_eq!(restart_policy.max_attempts, Some(3));
    }

    #[test]
    fn test_build_service_spec_minimal_config() {
        let orchestrator = SwarmOrchestrator {
            docker: Docker::connect_with_http_defaults().unwrap(),
            config: make_minimal_config(),
        };
        let connector = ApiConnector {
            id: "conn-2".to_string(),
            name: "Minimal Connector".to_string(),
            image: "opencti/connector-minimal:latest".to_string(),
            contract_hash: "hash456".to_string(),
            current_status: None,
            requested_status: "stopping".to_string(),
            contract_configuration: vec![],
        };
        let labels = HashMap::from([
            ("opencti-manager".to_string(), "mgr-1".to_string()),
            ("opencti-connector-id".to_string(), "conn-2".to_string()),
        ]);

        let spec = orchestrator.build_service_spec(&connector, labels);
        assert_eq!(spec.name, Some("minimal-connector".to_string()));
        assert!(spec.networks.is_none());
        assert!(spec.update_config.is_none());

        let task_template = spec.task_template.unwrap();
        assert!(task_template.placement.is_none());

        let container_spec = task_template.container_spec.unwrap();
        assert!(container_spec.hosts.is_none());
        assert!(container_spec.capability_add.is_none());
        assert!(container_spec.capability_drop.is_none());
        assert!(container_spec.sysctls.is_none());
        assert!(container_spec.dns_config.is_none());

        // Restart policy should still have defaults
        let restart_policy = task_template.restart_policy.unwrap();
        assert_eq!(restart_policy.condition, Some(TaskSpecRestartPolicyConditionEnum::ANY));
        assert_eq!(restart_policy.max_attempts, None);
    }
}
