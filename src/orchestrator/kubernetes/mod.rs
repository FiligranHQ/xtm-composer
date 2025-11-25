use crate::config::settings::Kubernetes;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;

pub mod kubernetes;
pub mod secret_refresher;

pub struct KubeOrchestrator {
    pub pods: Api<Pod>,
    pub deployments: Api<Deployment>,
    pub config: Kubernetes,
}
