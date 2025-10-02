use crate::config::settings::Kubernetes;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;

pub mod kubernetes;

pub struct KubeOrchestrator {
    pods: Api<Pod>,
    deployments: Api<Deployment>,
    config: Kubernetes,
}
