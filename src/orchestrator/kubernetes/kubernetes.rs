use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Kubernetes;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use crate::orchestrator::registry_resolver::RegistryResolver;
use async_trait::async_trait;
use k8s_openapi::DeepMerge;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, ContainerStatus, EnvVar, Pod, PodSpec, PodTemplateSpec, LocalObjectReference, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{DeleteParams, LogParams, Patch, PatchParams};
use kube::{
    Client,
    api::{Api, ListParams, PostParams, ResourceExt},
};
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, error, info, warn};
use base64::Engine;

impl KubeOrchestrator {
    pub async fn new(config: Kubernetes) -> Self {
        let client = match Client::try_default().await {
            Ok(client) => client,
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to create Kubernetes client"
                );
                panic!("Cannot initialize Kubernetes orchestrator without client");
            }
        };
        let pods: Api<Pod> = Api::default_namespaced(client.clone());
        let deployments: Api<Deployment> = Api::default_namespaced(client.clone());

        // Create imagePullSecret if registry credentials are configured
        let settings = crate::settings();
        let registry_config = &settings.opencti.daemon.registry;
        if registry_config.username.is_some() && registry_config.password.is_some() {
            Self::ensure_image_pull_secret(&client, registry_config)
                .await
                .expect("Failed to create imagePullSecret - cannot start without registry authentication");
        }
        
        Self {
            pods,
            deployments,
            config,
        }
    }

    /// Ensures the imagePullSecret exists with correct credentials.
    /// Checks if secret already exists with matching credentials before updating.
    /// This is called at orchestrator initialization if credentials are configured.
    async fn ensure_image_pull_secret(
        client: &Client,
        registry_config: &crate::config::settings::Registry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        const SECRET_NAME: &str = "opencti-registry-auth";
        let secrets: Api<Secret> = Api::default_namespaced(client.clone());

        // Step 1: Build expected Docker config JSON
        let auth_string = format!(
            "{}:{}",
            registry_config.username.as_ref()
                .map(|s| s.expose_secret())
                .unwrap_or(""),
            registry_config.password.as_ref()
                .map(|s| s.expose_secret())
                .unwrap_or("")
        );
        let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes());

        let server_url = registry_config.server.as_ref()
            .map(|s| s.as_str())
            .unwrap_or("https://index.docker.io/v1/");

        let docker_config = serde_json::json!({
            "auths": {
                server_url: {
                    "username": registry_config.username.as_ref()
                        .map(|s| s.expose_secret())
                        .unwrap_or(""),
                    "password": registry_config.password.as_ref()
                        .map(|s| s.expose_secret())
                        .unwrap_or(""),
                    "email": registry_config.email.as_ref()
                        .map(|s| s.expose_secret())
                        .unwrap_or(""),
                    "auth": auth_base64
                }
            }
        });

        let docker_config_bytes = serde_json::to_vec(&docker_config)?;

        // Step 2: Check if secret already exists with same credentials
        match secrets.get(SECRET_NAME).await {
            Ok(existing_secret) => {
                if let Some(existing_data) = existing_secret.data {
                    if let Some(existing_dockerconfig) = existing_data.get(".dockerconfigjson") {
                        // Compare the existing config with the new one
                        if existing_dockerconfig.0 == docker_config_bytes {
                            info!(
                                orchestrator = "kubernetes",
                                secret = SECRET_NAME,
                                "imagePullSecret already exists with correct credentials, skipping update"
                            );
                            return Ok(());
                        } else {
                            info!(
                                orchestrator = "kubernetes",
                                secret = SECRET_NAME,
                                "imagePullSecret exists but credentials differ, recreating"
                            );
                        }
                    }
                }
                // Delete existing secret with different credentials
                let _ = secrets.delete(SECRET_NAME, &DeleteParams::default()).await;
            }
            Err(_) => {
                info!(
                    orchestrator = "kubernetes",
                    secret = SECRET_NAME,
                    "imagePullSecret does not exist, creating new one"
                );
            }
        }

        // Step 3: Create the secret
        let mut secret_data = BTreeMap::new();
        secret_data.insert(
            ".dockerconfigjson".to_string(),
            k8s_openapi::ByteString(docker_config_bytes)
        );

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(SECRET_NAME.to_string()),
                ..Default::default()
            },
            type_: Some("kubernetes.io/dockerconfigjson".to_string()),
            data: Some(secret_data),
            ..Default::default()
        };

        info!(
            orchestrator = "kubernetes",
            secret = SECRET_NAME,
            server = server_url,
            "Creating imagePullSecret for private registry"
        );

        secrets.create(&PostParams::default(), &secret).await?;

        info!(
            orchestrator = "kubernetes",
            secret = SECRET_NAME,
            "Successfully created imagePullSecret"
        );

        Ok(())
    }

    pub fn get_image_pull_policy(&self) -> String {
        const VALID_POLICIES: [&str; 3] = ["Always", "IfNotPresent", "Never"];
        const DEFAULT_POLICY: &str = "IfNotPresent";
        
        match &self.config.image_pull_policy {
            Some(policy) if VALID_POLICIES.contains(&policy.as_str()) => {
                policy.clone()
            }
            Some(invalid_policy) => {
                warn!(
                    "Invalid image_pull_policy '{}'. Valid values: {:?}. Using default: {}",
                    invalid_policy, VALID_POLICIES, DEFAULT_POLICY
                );
                DEFAULT_POLICY.to_string()
            }
            None => {
                DEFAULT_POLICY.to_string()
            }
        }
    }

    pub fn container_envs(&self, connector: &ApiConnector) -> Vec<EnvVar> {
        let env_vars = connector.container_envs();
        env_vars
            .iter()
            .map(|config| EnvVar {
                name: config.key.clone(),
                value: Some(config.value.as_str().to_string()),
                value_from: None,
            })
            .collect()
    }


    async fn set_deployment_scale(&self, connector: &ApiConnector, scale: i32) {
        let deployment_patch = Deployment {
            spec: Some(DeploymentSpec {
                replicas: Some(scale),
                ..Default::default()
            }),
            ..Default::default()
        };
        let patch = Patch::Merge(&deployment_patch);
        let name = connector.container_name();
        match self.deployments
            .patch(name.as_str(), &PatchParams::default(), &patch)
            .await
        {
            Ok(_) => {
                debug!(
                    name = name,
                    scale = scale,
                    "Deployment scale updated"
                );
            }
            Err(e) => {
                error!(
                    name = name,
                    scale = scale,
                    error = %e,
                    "Failed to update deployment scale"
                );
            }
        }
    }

    pub fn from_deployment(deployment: Deployment) -> OrchestratorContainer {
        let dep = deployment.clone();
        let expected_replicas = dep.spec.unwrap().replicas.unwrap_or(0);
        let compute_state: &str = if expected_replicas == 0 {
            "terminated"
        } else {
            "running"
        };
        // Convert annotations (BTreeMap) to envs (HashMap)
        let annotations_as_env: HashMap<String, String> = deployment.annotations()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Convert labels (BTreeMap) to HashMap
        let labels_map: HashMap<String, String> = deployment.labels()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        
        OrchestratorContainer {
            id: deployment.uid().unwrap(),
            name: dep.metadata.name.unwrap(),
            state: compute_state.to_string(),
            envs: annotations_as_env,
            labels: labels_map,
            restart_count: 0, // Will be updated from pod status
            started_at: None, // Will be updated from pod status
        }
    }

    async fn get_deployment_pod(&self, connector_id: String) -> Option<Pod> {
        let lp = &ListParams::default().labels(&format!("opencti-connector-id={}", connector_id));
        let deployment_pods_response = self.pods.list(lp).await;
        match deployment_pods_response {
            Ok(pods) => {
                let pod_list = pods.items;
                match !pod_list.is_empty() {
                    true => pod_list.into_iter().next(),
                    false => None,
                }
            }
            Err(err) => {
                error!("Fail to get deployment pod: {}", err.to_string());
                None
            }
        }
    }

    pub fn build_configuration(
        &self,
        connector: &ApiConnector,
        labels: HashMap<String, String>,
    ) -> Deployment {
        self.build_configuration_with_image(
            connector,
            labels,
            &connector.image,
            Vec::new()
        )
    }

    pub fn build_configuration_with_image(
        &self,
        connector: &ApiConnector,
        labels: HashMap<String, String>,
        image_name: &str,
        image_pull_secrets: Vec<LocalObjectReference>,
    ) -> Deployment {
        let deployment_labels: BTreeMap<String, String> = labels.into_iter().collect();
        let pod_env = self.container_envs(connector);
        let is_starting = &connector.requested_status == "starting";
        
        let target_deployment = Deployment {
            metadata: ObjectMeta {
                name: Some(connector.container_name()),
                labels: Some(deployment_labels.clone()),
                // Specific case to let the hash config on top level
                annotations: Some(BTreeMap::from([(
                    "OPENCTI_CONFIG_HASH".into(),
                    connector.contract_hash.clone(),
                )])),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(if is_starting { 1 } else { 0 }),
                selector: LabelSelector {
                    match_labels: Some(deployment_labels.clone()),
                    ..Default::default()
                },
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(deployment_labels.clone()),
                        ..Default::default()
                    }),
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: connector.container_name(),
                            image: Some(image_name.to_string()),
                            env: Some(pod_env),
                            image_pull_policy: Some(self.get_image_pull_policy()),
                            ..Default::default()
                        }],
                        image_pull_secrets: if image_pull_secrets.is_empty() {
                            None
                        } else {
                            Some(image_pull_secrets)
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut base_deploy = self.config.base_deployment.clone();
        // No direct deploy configuration, check the json format
        if base_deploy.is_none() {
            if let Some(json_deploy) = self.config.base_deployment_json.clone() {
                // If json base deploy defined, try to generate the base from it
                match serde_json::from_str(json_deploy.as_str()) {
                    Ok(deployment) => {
                        base_deploy = Some(deployment);
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            "Failed to parse base_deployment_json, using default deployment"
                        );
                    }
                }
            }
        }
        let mut base_deployment = base_deploy.unwrap_or(Deployment {
            ..Default::default()
        });
        base_deployment.merge_from(target_deployment);
        base_deployment
    }

    fn enrich_container_from_pod(&self, container: &mut OrchestratorContainer, pod: Pod) {
        let container_status = pod.status
            .and_then(|status| status.container_statuses)
            .and_then(|statuses| statuses.first().cloned());
        
        if let Some(status) = container_status {
            container.restart_count = status.restart_count as u32;
            
            if let Some(started_at) = self.extract_started_at(&status) {
                container.started_at = Some(started_at);
            }
        }
    }
    
    fn extract_started_at(&self, container_status: &ContainerStatus) -> Option<String> {
        container_status.state
            .as_ref()
            .and_then(|state| state.running.as_ref())
            .and_then(|running| running.started_at.as_ref())
            .map(|timestamp| timestamp.0.to_rfc3339())
    }

    /// Helper to log deployment operations consistently
    fn log_deployment(&self, operation: &str, status: &str, image: &str, registry: Option<&str>) {
        let registry_type = registry
            .map(|_| "private registry")
            .unwrap_or("Docker Hub");
        
        info!(
            orchestrator = "kubernetes",
            image = image,
            registry = registry.unwrap_or("docker.io"),
            operation = operation,
            status = status,
            "Kubernetes deployment: {} image from {}",
            status, registry_type
        );
    }
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {
    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let deployment = match self
            .deployments
            .get(connector.container_name().as_str())
            .await
        {
            Ok(dep) => dep,
            Err(err) => {
                debug!("Cant find deployment: {}", err.to_string());
                return None;
            }
        };
        
        let mut container = KubeOrchestrator::from_deployment(deployment);
        
        // Enrich container with pod information
        if let Some(pod) = self.get_deployment_pod(connector.id.clone()).await {
            self.enrich_container_from_pod(&mut container, pod);
        }
        
        Some(container)
    }

    async fn list(&self) -> Vec<OrchestratorContainer> {
        let settings = crate::settings();
        let lp = &ListParams::default()
            .labels(&format!("opencti-manager={}", settings.manager.id.clone()));
        match self.deployments.list(lp).await {
            Ok(get_deployments) => {
                get_deployments
                    .into_iter()
                    .map(|deployment| KubeOrchestrator::from_deployment(deployment))
                    .collect()
            }
            Err(e) => {
                error!(
                    error = %e,
                    manager_id = %settings.manager.id,
                    "Failed to list Kubernetes deployments"
                );
                Vec::new()
            }
        }
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) {
        connector.display_env_variables();
        self.set_deployment_scale(connector, 1).await;
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) {
        self.set_deployment_scale(connector, 0).await;
    }

    async fn remove(&self, container: &OrchestratorContainer) {
        let lp = &ListParams::default().labels(&format!(
            "opencti-connector-id={}",
            container.extract_opencti_id()
        ));
        let dp = &DeleteParams::default();
        let delete_response = self.deployments.delete_collection(dp, lp).await;
        match delete_response {
            Ok(_) => info!(
                "Deployment successfully deleted: {}",
                container.extract_opencti_id()
            ),
            Err(err) => error!("Fail removing the deployments: {}", err.to_string()),
        }
    }

    async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let labels = self.labels(connector);
        let deployment_patch = self.build_configuration(connector, labels);
        let patch = Patch::Merge(&deployment_patch);
        let name = connector.container_name();
        let deployment_result = self
            .deployments
            .patch(name.as_str(), &PatchParams::default(), &patch)
            .await;
        match deployment_result {
            Ok(deployment) => Some(KubeOrchestrator::from_deployment(deployment)),
            Err(kube::Error::Api(ae)) => {
                error!("Kubernetes update api error: {}", ae.to_string());
                None
            }
            Err(e) => {
                error!("Kubernetes update unknown error: {}", e.to_string());
                None
            }
        }
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let settings = crate::settings();
        
        // Create registry resolver
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = RegistryResolver::new(registry_config);
        
        // Resolve image name with registry prefix if needed
        let resolved_image = match resolver.resolve_image(&connector.image) {
            Ok(resolved) => resolved,
            Err(e) => {
                error!("Failed to resolve image name {}: {}", connector.image, e);
                return None;
            }
        };

        // Use pre-existing imagePullSecret if authentication is needed
        const SECRET_NAME: &str = "opencti-registry-auth";
        let image_pull_secrets = if resolved_image.needs_auth {
            info!(
                orchestrator = "kubernetes",
                secret = SECRET_NAME,
                image = resolved_image.full_name,
                "Using pre-existing imagePullSecret for private registry"
            );
            vec![LocalObjectReference {
                name: SECRET_NAME.to_string()
            }]
        } else {
            Vec::new()
        };

        // Build deployment configuration with resolved image
        let labels = self.labels(connector);
        let deployment_creation = self.build_configuration_with_image(
            connector, 
            labels, 
            &resolved_image.full_name,
            image_pull_secrets
        );

        // Log deployment attempt
        self.log_deployment("deploy", "started", &resolved_image.full_name, resolved_image.registry_server.as_deref());

        match self
            .deployments
            .create(&PostParams::default(), &deployment_creation)
            .await
        {
            Ok(deployment) => {
                self.log_deployment("deploy", "completed", &resolved_image.full_name, resolved_image.registry_server.as_deref());
                Some(KubeOrchestrator::from_deployment(deployment))
            }
            Err(kube::Error::Api(ae)) => {
                error!(
                    orchestrator = "kubernetes",
                    image = resolved_image.full_name,
                    operation = "deploy",
                    status = "failed",
                    error = %ae,
                    "Kubernetes deployment failed (API error)"
                );
                None
            }
            Err(e) => {
                error!(
                    orchestrator = "kubernetes",
                    image = resolved_image.full_name,
                    operation = "deploy",
                    status = "failed",
                    error = %e,
                    "Kubernetes deployment failed"
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
        let deployment_pod = self.get_deployment_pod(connector.id.clone()).await;
        match deployment_pod {
            Some(pod) => {
                let lp = LogParams::default();
                let node_name = pod.metadata.name.unwrap();
                let text_logs_response = self.pods.logs(node_name.as_str(), &lp).await;
                match text_logs_response {
                    Ok(text_logs) => Some(text_logs.lines().map(|line| line.to_string()).collect()),
                    Err(err) => {
                        error!("Error fetching logs: {}", err.to_string());
                        None
                    }
                }
            }
            None => None,
        }
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus {
        match container.state.as_str() {
            "running" => ConnectorStatus::Started,
            "waiting" => ConnectorStatus::Started,
            "exited" => ConnectorStatus::Stopped,
            "terminated" => ConnectorStatus::Stopped,
            _ => ConnectorStatus::Stopped,
        }
    }
}
