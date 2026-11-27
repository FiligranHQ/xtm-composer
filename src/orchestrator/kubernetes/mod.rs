use crate::config::settings::{Kubernetes, Registry};
use crate::orchestrator::LogHistory;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::Api;

pub mod kubernetes;

pub struct KubeOrchestrator {
    pods: Api<Pod>,
    deployments: Api<Deployment>,
    secrets: Api<Secret>,
    config: Kubernetes,
    registry: Option<Registry>,
    platform: &'static str,
    /// Last lines of each workload's previous pod, kept across a pod replacement
    log_history: LogHistory,
}
