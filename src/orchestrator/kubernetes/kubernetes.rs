use crate::api::{ApiConnector, ConnectorStatus};
use crate::config::settings::Kubernetes;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use k8s_openapi::DeepMerge;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, ContainerStatus, EnvVar, Pod, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{DeleteParams, LogParams, Patch, PatchParams};
use kube::{
    Client,
    api::{Api, ListParams, PostParams, ResourceExt},
};
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, error, info};

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
                error!(error = err.to_string(), "Fail to get deployment pod");
                None
            }
        }
    }

    pub fn build_configuration(
        &self,
        connector: &ApiConnector,
        labels: HashMap<String, String>,
    ) -> Deployment {
        let deployment_labels: BTreeMap<String, String> = labels.into_iter().collect();
        let pod_env = self.container_envs(connector);
        let is_starting = connector.requested_status.clone().eq("starting");
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
                            image: Some(connector.image.clone()),
                            env: Some(pod_env),
                            image_pull_policy: Some("IfNotPresent".into()),
                            ..Default::default()
                        }],
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
                debug!(error = err.to_string(), "Cant find deployment");
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

    async fn start(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        connector.display_env_variables();
        self.set_deployment_scale(connector, 1).await;
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        self.set_deployment_scale(connector, 0).await;
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        let lp = &ListParams::default().labels(&format!(
            "opencti-connector-id={}",
            container.extract_opencti_id()
        ));
        let dp = &DeleteParams::default();
        let delete_response = self.deployments.delete_collection(dp, lp).await;
        match delete_response {
            Ok(_) => info!(
                id = container.extract_opencti_id(),
                "Deployment successfully deleted"
            ),
            Err(err) => error!(error = err.to_string(), "Fail removing the deployments"),
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
                error!(error = ae.to_string(), "Kubernetes update api error");
                None
            }
            Err(e) => {
                error!(error = e.to_string(), "Kubernetes update unknown error");
                None
            }
        }
    }

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        let labels = self.labels(connector);
        let deployment_creation = self.build_configuration(connector, labels);
        match self
            .deployments
            .create(&PostParams::default(), &deployment_creation)
            .await
        {
            Ok(deployment) => Some(KubeOrchestrator::from_deployment(deployment)),
            Err(kube::Error::Api(ae)) => {
                error!(error = ae.to_string(), "Kubernetes creation api error");
                None
            }
            Err(e) => {
                error!(error = e.to_string(), "Kubernetes creation unknown error");
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
                        error!(error = err.to_string(), "Error fetching logs");
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
