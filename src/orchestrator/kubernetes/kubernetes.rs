use crate::api::connector::{ConnectorCurrentStatus, ManagedConnector};
use crate::config::settings::{Kubernetes, Settings};
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, EnvVar, Pod, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{DeleteParams, LogParams, Patch, PatchParams};
use kube::{
    api::{Api, ListParams, PostParams, ResourceExt},
    Client,
};
use log::{error, info};
use std::collections::{BTreeMap, HashMap};

impl KubeOrchestrator {
    pub async fn new(_config: &Kubernetes) -> Self {
        let client = Client::try_default().await.unwrap();
        let pods: Api<Pod> = Api::default_namespaced(client.clone());
        let deployments: Api<Deployment> = Api::default_namespaced(client.clone());
        Self { pods, deployments }
    }

    pub fn container_envs(&self, settings: &Settings, connector: &ManagedConnector) -> Vec<EnvVar> {
        let env_vars = connector.container_envs(settings, connector);
        env_vars
            .iter()
            .map(|config| EnvVar {
                name: config.key.clone(),
                value: Some(config.value.clone()),
                value_from: None,
            })
            .collect()
    }

    pub fn compute_pod_status(pod: &Pod) -> String {
        let pod_container_state = pod
            .clone()
            .status
            .unwrap()
            .container_statuses
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .clone()
            .state
            .unwrap();
        let status = match (
            pod_container_state.waiting,
            pod_container_state.running,
            pod_container_state.terminated,
        ) {
            (Some(_waiting_status), None, None) => "waiting",
            (None, Some(_running_status), None) => "running",
            (None, None, Some(_terminated_status)) => "terminated",
            _ => "exited",
        };
        status.to_string()
    }

    pub fn convert_to_map(labels: &BTreeMap<String, String>) -> HashMap<String, String> {
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    async fn set_deployment_scale(&self, connector: &ManagedConnector, scale: i32) -> () {
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

    pub fn from_pod(deployment: Deployment, pod: &Pod) -> OrchestratorContainer {
        let pod_spec = pod.spec.clone().unwrap();
        let pod_container = pod_spec.containers.into_iter().next().unwrap();
        let mut annotations_as_env = KubeOrchestrator::convert_to_map(deployment.annotations());
        pod_container
            .env
            .unwrap_or_default()
            .iter()
            .for_each(|env| {
                annotations_as_env.insert(env.clone().name, env.clone().value.unwrap_or_default());
            });
        OrchestratorContainer {
            id: pod.uid().unwrap(),
            state: KubeOrchestrator::compute_pod_status(&pod),
            // image: pod_container.image.clone().unwrap(),
            envs: annotations_as_env,
            labels: KubeOrchestrator::convert_to_map(&pod.labels()),
        }
    }

    pub fn from_deployment(deployment: Deployment) -> OrchestratorContainer {
        // let deployment_spec = deployment.spec.clone().unwrap();
        // let template_spec = deployment_spec.template.clone();
        // let spec = template_spec.spec.unwrap();
        // let pod_container = spec.containers.iter().next().unwrap();
        let annotations_as_env = KubeOrchestrator::convert_to_map(deployment.annotations());
        OrchestratorContainer {
            id: deployment.uid().unwrap(),
            state: "exited".to_string(),
            // image: pod_container.image.clone().unwrap(),
            envs: annotations_as_env,
            labels: KubeOrchestrator::convert_to_map(&deployment.labels()),
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
            Err(_) => None,
        }
    }

    pub fn build_configuration(
        &self,
        settings: &Settings,
        connector: &ManagedConnector,
        labels: HashMap<String, String>,
    ) -> Deployment {
        let deployment_labels: BTreeMap<String, String> = labels.into_iter().collect();
        let pod_env = self.container_envs(settings, connector);
        let is_starting = connector
            .manager_requested_status
            .clone()
            .unwrap()
            .eq("starting");
        Deployment {
            metadata: ObjectMeta {
                name: Some(connector.container_name()),
                labels: Some(deployment_labels.clone()),
                // Specific case to let the hash config on top level
                annotations: Some(BTreeMap::from([(
                    "OPENCTI_CONFIG_HASH".into(),
                    connector.manager_contract_hash.clone().unwrap(),
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
                            image: connector.manager_contract_image.clone(),
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
        }
    }
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {
    async fn get(&self, connector: &ManagedConnector) -> Option<OrchestratorContainer> {
        let get_deployment = self
            .deployments
            .get(connector.container_name().as_str())
            .await;
        match get_deployment {
            Ok(deployment) => {
                // Looking for an existing pod.
                let pod = self
                    .get_deployment_pod(connector.id.clone().into_inner())
                    .await;
                if pod.is_some() {
                    // If pod exists, return container based on the pod
                    Some(KubeOrchestrator::from_pod(deployment, &pod.unwrap()))
                } else {
                    // If not, return container based on the deployment
                    Some(KubeOrchestrator::from_deployment(deployment))
                }
            }
            Err(_) => None,
        }
    }

    async fn list(&self, settings: &Settings) -> Option<Vec<OrchestratorContainer>> {
        let lp = &ListParams::default()
            .labels(&format!("opencti-manager={}", settings.manager.id.clone()));
        let get_deployments = self.deployments.list(lp).await.unwrap();
        Some(
            get_deployments
                .into_iter()
                .map(|deployment| KubeOrchestrator::from_deployment(deployment))
                .collect(),
        )
    }

    async fn start(&self, _container: &OrchestratorContainer, connector: &ManagedConnector) -> () {
        self.set_deployment_scale(connector, 1).await;
    }

    async fn stop(&self, _container: &OrchestratorContainer, connector: &ManagedConnector) -> () {
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
                "Deployment {} successfully deleted",
                container.extract_opencti_id()
            ),
            Err(_) => error!("Fail removing the deployments"),
        }
    }

    async fn refresh(
        &self,
        settings: &Settings,
        connector: &ManagedConnector,
    ) -> Option<OrchestratorContainer> {
        let labels = self.labels(settings, connector);
        let deployment_patch = self.build_configuration(settings, connector, labels);
        let patch = Patch::Merge(&deployment_patch);
        let name = connector.container_name();
        let deployment_result = self
            .deployments
            .patch(name.as_str(), &PatchParams::default(), &patch)
            .await;
        match deployment_result {
            Ok(deployment) => Some(KubeOrchestrator::from_deployment(deployment)),
            Err(kube::Error::Api(ae)) => {
                error!("Kubernetes update api error {:?}", ae);
                None
            }
            Err(e) => {
                error!("Kubernetes update unknown error {:?}", e);
                None
            }
        }
    }

    async fn deploy(
        &self,
        settings: &Settings,
        connector: &ManagedConnector,
    ) -> Option<OrchestratorContainer> {
        let labels = self.labels(settings, connector);
        let deployment_creation = self.build_configuration(settings, connector, labels);
        match self
            .deployments
            .create(&PostParams::default(), &deployment_creation)
            .await
        {
            Ok(deployment) => Some(KubeOrchestrator::from_deployment(deployment)),
            Err(kube::Error::Api(ae)) => {
                error!("Kubernetes creation api error {:?}", ae);
                None
            }
            Err(e) => {
                error!("Kubernetes creation unknown error {:?}", e);
                None
            }
        }
    }

    async fn logs(
        &self,
        _container: &OrchestratorContainer,
        connector: &ManagedConnector,
    ) -> Option<Vec<String>> {
        let deployment_pod = self
            .get_deployment_pod(connector.id.clone().into_inner())
            .await;
        match deployment_pod {
            Some(pod) => {
                let lp = LogParams::default();
                let node_name = pod.metadata.name.unwrap();
                let text_logs_response = self.pods.logs(node_name.as_str(), &lp).await;
                match text_logs_response {
                    Ok(text_logs) => Some(text_logs.lines().map(|line| line.to_string()).collect()),
                    Err(_) => None,
                }
            }
            None => None,
        }
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus {
        match container.state.as_str() {
            "running" => ConnectorCurrentStatus::Started,
            "waiting" => ConnectorCurrentStatus::Started,
            "exited" => ConnectorCurrentStatus::Stopped,
            "terminated" => ConnectorCurrentStatus::Stopped,
            _ => ConnectorCurrentStatus::Stopped,
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
