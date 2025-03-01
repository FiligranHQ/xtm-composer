use crate::api::connector::{Connector, ConnectorCurrentStatus};
use crate::config::settings::{Kubernetes, Settings};
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{Container, EnvVar, Pod, PodSpec, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{LogParams, Patch, PatchParams};
use kube::{
    api::{Api, ListParams, PostParams, ResourceExt},
    Client,
};
use log::error;
use std::collections::{BTreeMap, HashMap};

impl KubeOrchestrator {
    pub async fn new(_config: &Kubernetes) -> Self {
        let client = Client::try_default().await.unwrap();
        let pods: Api<Pod> = Api::default_namespaced(client.clone());
        let deployments: Api<Deployment> = Api::default_namespaced(client.clone());
        Self { pods, deployments }
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

    pub fn compute_pod_labels(labels: &BTreeMap<String, String>) -> HashMap<String, String> {
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn from_pod(pod: &Pod) -> OrchestratorContainer {
        let pod_spec = pod.spec.clone().unwrap();
        let pod_container = pod_spec.containers.iter().next().unwrap();
        OrchestratorContainer {
            id: pod.uid().unwrap(),
            state: KubeOrchestrator::compute_pod_status(&pod),
            image: pod_container.image.clone().unwrap(),
            labels: KubeOrchestrator::compute_pod_labels(&pod.labels()),
        }
    }

    pub fn from_deployment(deployment: &Deployment) -> OrchestratorContainer {
        let deployment_spec = deployment.spec.clone().unwrap();
        let template_spec = deployment_spec.template.clone();
        let spec = template_spec.spec.unwrap();
        let pod_container = spec.containers.iter().next().unwrap();
        OrchestratorContainer {
            id: deployment.uid().unwrap(),
            state: "exited".to_string(),
            image: pod_container.image.clone().unwrap(),
            labels: KubeOrchestrator::compute_pod_labels(&deployment.labels()),
        }
    }

    async fn get_deployment_pod(&self, connector: &Connector) -> Option<Pod> {
        let lp = &ListParams::default()
            .labels(&format!("opencti-connector-id={}", connector.id.inner()));
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

    async fn set_deployment_scale(&self, connector: &Connector, scale: i32) -> () {
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
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {
    async fn container(
        &self,
        _container_id: String,
        connector: &Connector,
    ) -> Option<OrchestratorContainer> {
        let deployment_pod = self.get_deployment_pod(connector).await;
        match deployment_pod {
            Some(pod) => Some(KubeOrchestrator::from_pod(&pod)),
            None => None,
        }
    }

    async fn containers(&self, connector: &Connector) -> Option<Vec<OrchestratorContainer>> {
        let lp = &ListParams::default()
            .labels(&format!("opencti-connector-id={}", connector.id.inner()));
        let containers: Vec<OrchestratorContainer> = self
            .deployments
            .list(lp)
            .await
            .unwrap()
            .iter()
            .map(|deployment| KubeOrchestrator::from_deployment(deployment))
            .collect();
        Some(containers)
    }

    async fn container_start(
        &self,
        _container: &OrchestratorContainer,
        connector: &Connector,
    ) -> () {
        self.set_deployment_scale(connector, 1).await;
    }

    async fn container_stop(
        &self,
        _container: &OrchestratorContainer,
        connector: &Connector,
    ) -> () {
        self.set_deployment_scale(connector, 0).await;
    }

    async fn container_deploy(
        &self,
        _settings: &Settings,
        connector: &Connector,
    ) -> Option<OrchestratorContainer> {
        let mut deployment_labels: BTreeMap<String, String> = BTreeMap::new();
        deployment_labels.insert(
            "opencti-connector-id".into(),
            connector.id.clone().into_inner(),
        );
        let pod_env: Vec<EnvVar> = connector
            .clone()
            .manager_contract_configuration
            .unwrap()
            .iter()
            .map(|conf| EnvVar {
                name: conf.key.clone(),
                value: Some(conf.value.clone()),
                value_from: None,
            })
            .collect();

        let deployment_creation = Deployment {
            metadata: ObjectMeta {
                name: Some(connector.container_name()),
                labels: Some(deployment_labels.clone()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(0),
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
        };

        match self
            .deployments
            .create(&PostParams::default(), &deployment_creation)
            .await
        {
            Ok(deployment) => Some(self.container(deployment.uid()?, connector).await.unwrap()),
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

    async fn container_logs(
        &self,
        _container: &OrchestratorContainer,
        connector: &Connector,
    ) -> Option<Vec<String>> {
        let deployment_pod = self.get_deployment_pod(connector).await;
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
