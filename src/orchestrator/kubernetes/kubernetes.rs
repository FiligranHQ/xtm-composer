use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Kubernetes;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use crate::orchestrator::registry_resolver::RegistryResolver;
use async_trait::async_trait;
use k8s_openapi::DeepMerge;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, ContainerStatus, EnvVar, Pod, PodSpec, PodTemplateSpec, Secret, LocalObjectReference};
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
        let client = Client::try_default().await.unwrap();
        let pods: Api<Pod> = Api::default_namespaced(client.clone());
        let deployments: Api<Deployment> = Api::default_namespaced(client.clone());
        Self {
            pods,
            deployments,
            config,
        }
    }

    // Generate RFC 1123 compliant secret name
    fn generate_secret_name(&self, registry_config: &crate::config::settings::Registry) -> String {
        let default_server = "default".to_string();
        let server = registry_config.server.as_ref()
            .unwrap_or(&default_server);
        
        // Remove trailing slashes and normalize the server name
        let normalized_server = server.trim_end_matches('/');
        
        // Replace invalid characters with hyphens
        let mut secret_name = format!("opencti-registry-{}", 
            normalized_server.replace([':', '.', '/'], "-"));
        
        // Ensure the name doesn't end with a hyphen (RFC 1123 violation)
        while secret_name.ends_with('-') {
            secret_name.pop();
        }
        
        // Ensure the name starts with alphanumeric character
        if secret_name.starts_with('-') || secret_name.starts_with('.') {
            secret_name = format!("r{}", secret_name);
        }
        
        // Limit length to 253 characters (Kubernetes limit)
        if secret_name.len() > 253 {
            secret_name.truncate(253);
            // Ensure it still doesn't end with a hyphen after truncation
            while secret_name.ends_with('-') {
                secret_name.pop();
            }
        }
        
        secret_name
    }
    
    // Validate secret name against RFC 1123 subdomain rules
    fn validate_secret_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Secret name cannot be empty".to_string());
        }
        
        if name.len() > 253 {
            return Err(format!("Secret name too long: {} characters (max 253)", name.len()));
        }
        
        // Check first character
        let first_char = name.chars().next().unwrap();
        if !first_char.is_ascii_alphanumeric() {
            return Err("Secret name must start with alphanumeric character".to_string());
        }
        
        // Check last character
        let last_char = name.chars().last().unwrap();
        if !last_char.is_ascii_alphanumeric() {
            return Err("Secret name must end with alphanumeric character".to_string());
        }
        
        // Check all characters
        for (i, c) in name.char_indices() {
            if !c.is_ascii_alphanumeric() && c != '-' && c != '.' {
                return Err(format!("Invalid character '{}' at position {}", c, i));
            }
        }
        
        Ok(())
    }

    // Create or update imagePullSecret for private registry
    async fn ensure_image_pull_secret(&self, registry_config: &crate::config::settings::Registry) -> Result<String, Box<dyn std::error::Error>> {
        let client = Client::try_default().await?;
        let secrets: Api<Secret> = Api::default_namespaced(client);
        
        // Generate RFC 1123 compliant secret name
        let secret_name = self.generate_secret_name(registry_config);
        
        // Validate secret name compliance
        if let Err(e) = self.validate_secret_name(&secret_name) {
            return Err(format!("Invalid secret name '{}': {}", secret_name, e).into());
        }

        // Create Docker config JSON
        let auth_string = format!(
            "{}:{}",
            registry_config.username.as_ref().unwrap_or(&String::new()),
            registry_config.password.as_ref().unwrap_or(&String::new())
        );
        let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes());
        
        let default_server = "https://index.docker.io/v1/".to_string();
        let server_url = registry_config.server.as_ref().unwrap_or(&default_server);
        let docker_config = serde_json::json!({
            "auths": {
                server_url: {
                    "username": registry_config.username,
                    "password": registry_config.password,
                    "email": registry_config.email.as_ref().unwrap_or(&"".to_string()),
                    "auth": auth_base64
                }
            }
        });
        
        // Serialize Docker config JSON to bytes directly
        let docker_config_bytes = serde_json::to_vec(&docker_config)
            .map_err(|e| format!("Failed to serialize Docker config JSON: {}", e))?;

        // Create secret with raw JSON bytes (ByteString will base64 encode them)
        let mut secret_data = BTreeMap::new();
        secret_data.insert(
            ".dockerconfigjson".to_string(), 
            k8s_openapi::ByteString(docker_config_bytes)
        );

        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(secret_name.clone()),
                ..Default::default()
            },
            type_: Some("kubernetes.io/dockerconfigjson".to_string()),
            data: Some(secret_data),
            ..Default::default()
        };

        // Try to create or update the secret
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => {
                info!("Created imagePullSecret: {}", secret_name);
                Ok(secret_name)
            }
            Err(kube::Error::Api(api_err)) if api_err.code == 409 => {
                // Secret already exists, update it
                match secrets.replace(&secret_name, &PostParams::default(), &secret).await {
                    Ok(_) => {
                        debug!("Updated existing imagePullSecret: {}", secret_name);
                        Ok(secret_name)
                    }
                    Err(e) => {
                        error!("Failed to update imagePullSecret: {}", e);
                        Err(e.into())
                    }
                }
            }
            Err(e) => {
                error!("Failed to create imagePullSecret: {}", e);
                Err(e.into())
            }
        }
    }

    // Validate and return image pull policy
    fn get_image_pull_policy(&self) -> String {
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
                value: Some(config.value.clone()),
                value_from: None,
            })
            .collect()
    }

    pub fn convert_to_map(labels: &BTreeMap<String, String>) -> HashMap<String, String> {
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
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
        self.deployments
            .patch(name.as_str(), &PatchParams::default(), &patch)
            .await
            .unwrap();
    }

    pub fn from_deployment(deployment: Deployment) -> OrchestratorContainer {
        let dep = deployment.clone();
        let expected_replicas = dep.spec.unwrap().replicas.unwrap_or(0);
        let compute_state: &str = if expected_replicas == 0 {
            "terminated"
        } else {
            "running"
        };
        let annotations_as_env = KubeOrchestrator::convert_to_map(deployment.annotations());
        OrchestratorContainer {
            id: deployment.uid().unwrap(),
            name: dep.metadata.name.unwrap(),
            state: compute_state.to_string(),
            envs: annotations_as_env,
            labels: KubeOrchestrator::convert_to_map(&deployment.labels()),
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
            let json_deploy = self.config.base_deployment_json.clone();
            // If json base deploy defined, try to generate the base from it
            if json_deploy.is_some() {
                base_deploy = Some(serde_json::from_str(json_deploy.unwrap().as_str()).unwrap());
            }
        }
        let mut base_deployment = base_deploy.unwrap_or(Deployment {
            ..Default::default()
        });
        base_deployment.merge_from(target_deployment);
        base_deployment
    }

    // Enrich container with pod information
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
    
    // Extract started_at timestamp from container status
    fn extract_started_at(&self, container_status: &ContainerStatus) -> Option<String> {
        container_status.state
            .as_ref()
            .and_then(|state| state.running.as_ref())
            .and_then(|running| running.started_at.as_ref())
            .map(|timestamp| timestamp.0.to_rfc3339())
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
        let get_deployments = self.deployments.list(lp).await.unwrap();
        get_deployments
            .into_iter()
            .map(|deployment| KubeOrchestrator::from_deployment(deployment))
            .collect()
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
        
        // Create registry resolver - use daemon-level registry config
        let registry_config = settings.opencti.daemon.registry.clone();
        let resolver = RegistryResolver::new(registry_config.clone());
        
        // Resolve image name with registry prefix if needed
        let resolved_image = match resolver.resolve_image(&connector.image) {
            Ok(resolved) => resolved,
            Err(e) => {
                error!("Failed to resolve image name {}: {}", connector.image, e);
                return None;
            }
        };

        // Handle private registry authentication if needed
        let mut image_pull_secrets = Vec::new();
        if resolved_image.needs_auth {
            if let Some(registry_config) = &registry_config {
                // Ensure authentication is cached (similar to Docker)
                if let Some(registry_server) = &resolved_image.registry_server {
                    match resolver.get_credentials(registry_server).await {
                        Ok(_) => {
                            info!("Registry authentication validated: {}", registry_server);
                            
                            // Create imagePullSecret
                            match self.ensure_image_pull_secret(registry_config).await {
                                Ok(secret_name) => {
                                    image_pull_secrets.push(LocalObjectReference {
                                        name: secret_name
                                    });
                                }
                                Err(e) => {
                                    error!("Failed to create imagePullSecret for registry {}: {}", registry_server, e);
                                    return None;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to get registry credentials for registry {}: {}", registry_server, e);
                            return None;
                        }
                    }
                }
            }
        }

        // Build deployment configuration with resolved image
        let labels = self.labels(connector);
        let deployment_creation = self.build_configuration_with_image(
            connector, 
            labels, 
            &resolved_image.full_name,
            image_pull_secrets
        );

        // Log deployment attempt
        if resolved_image.registry_server.is_some() {
            info!("Deploying Kubernetes pod with image {} from private registry", resolved_image.full_name);
        } else {
            info!("Deploying Kubernetes pod with image {} from Docker Hub", resolved_image.full_name);
        }

        match self
            .deployments
            .create(&PostParams::default(), &deployment_creation)
            .await
        {
            Ok(deployment) => Some(KubeOrchestrator::from_deployment(deployment)),
            Err(kube::Error::Api(ae)) => {
                error!("Kubernetes creation api error: {}", ae.to_string());
                None
            }
            Err(e) => {
                error!("Kubernetes creation unknown error: {}", e.to_string());
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

// region async map resolution code sample
// let async_resolver = get_deployments
//     .into_iter()
//     .map(|deployment| self.get_container(deployment, connector));
// let deploy_to_containers = futures::stream::iter(async_resolver)
//     .buffer_unordered(3)
//     .collect::<Vec<_>>();
// Some(deploy_to_containers.await)
// endregion
