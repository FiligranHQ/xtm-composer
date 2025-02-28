// TODO Remove macro after implementation
#![allow(unused_variables)]

use crate::api::connector::{Connector, ConnectorCurrentStatus};
use crate::config::settings::{Kubernetes, Settings};
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use k8s_openapi::api::core::v1::{Container, EnvVar, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::LogParams;
use kube::{
    api::{Api, ListParams, PostParams, ResourceExt},
    Client,
};
use log::error;
use std::collections::BTreeMap;

impl KubeOrchestrator {
    pub async fn new(config: &Kubernetes) -> Self {
        let base_uri = config.api.clone();
        // let config = Config::new(cluster_url).await.unwrap();
        let client = Client::try_default().await.unwrap();
        Self { base_uri, client }
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
            (Some(waiting_status), None, None) => "waiting",
            (None, Some(running_status), None) => "running",
            (None, None, Some(terminated_status)) => "terminated",
            _ => "exited",
        };
        status.to_string()
    }
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {
    async fn container(
        &self,
        container_id: String,
        connector: &Connector,
    ) -> Option<OrchestratorContainer> {
        todo!("kubernetes container")
    }

    async fn containers(&self) -> Option<Vec<OrchestratorContainer>> {
        let pods: Api<Pod> = Api::default_namespaced(self.client.clone());
        let mut containers_get: Vec<OrchestratorContainer> = Vec::new();
        for pod in pods.list(&ListParams::default()).await.unwrap() {
            let inner_pod = pod.clone();
            let pod_spec = inner_pod.spec.clone()?;
            let pod_container = pod_spec.containers.iter().next().unwrap();
            let pod_labels = pod
                .labels()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            containers_get.insert(
                0,
                OrchestratorContainer {
                    id: inner_pod.uid().unwrap(),
                    state: KubeOrchestrator::compute_pod_status(&pod),
                    image: pod_container.image.clone()?,
                    labels: pod_labels,
                },
            );
        }
        Some(
            containers_get
                .into_iter()
                .filter(|c: &OrchestratorContainer| c.is_managed())
                .collect(),
        )
    }

    async fn container_start(
        &self,
        container: &OrchestratorContainer,
        connector: &Connector,
    ) -> () {
        todo!("kubernetes start")
    }

    async fn container_stop(&self, container: &OrchestratorContainer, connector: &Connector) -> () {
        todo!("kubernetes stop")
    }

    async fn container_deploy(
        &self,
        settings: &Settings,
        connector: &Connector,
    ) -> Option<OrchestratorContainer> {
        let pods: Api<Pod> = Api::default_namespaced(self.client.clone());
        let mut pod_labels: BTreeMap<String, String> = BTreeMap::new();
        pod_labels.insert(
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
        let pod_creation = Pod {
            metadata: ObjectMeta {
                name: Some(connector.container_name()),
                labels: Some(pod_labels),
                ..Default::default()
            },
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
        };
        match pods.create(&PostParams::default(), &pod_creation).await {
            Ok(pod) => {
                let inner_pod = pod.clone();
                let pod_spec = inner_pod.spec.clone()?;
                let pod_container = pod_spec.containers.iter().next().unwrap();
                let pod_labels = pod
                    .labels()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Some(OrchestratorContainer {
                    id: inner_pod.uid().unwrap(),
                    state: KubeOrchestrator::compute_pod_status(&pod),
                    image: pod_container.image.clone()?,
                    labels: pod_labels,
                })
            }
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
        container: &OrchestratorContainer,
        connector: &Connector,
    ) -> Vec<String> {
        let pods: Api<Pod> = Api::default_namespaced(self.client.clone());
        let lp = LogParams::default();
        let text_logs = pods
            .logs(connector.container_name().clone().as_str(), &lp)
            .await
            .unwrap();
        text_logs.lines().map(|line| line.to_string()).collect()
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
