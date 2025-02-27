use async_trait::async_trait;
use crate::api::connector::{Connector};
use crate::config::settings::{Kubernetes};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use crate::orchestrator::kubernetes::KubeOrchestrator;

impl KubeOrchestrator {
    pub fn new(config: &Kubernetes) -> Self {
        let base_uri = config.api.clone();
        Self { base_uri }
    }
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {

    async fn container(&self, container_id: String) -> Option<OrchestratorContainer> {
        todo!("kubernetes container")
    }

    async fn containers(&self) -> Option<Vec<OrchestratorContainer>> {
        todo!("kubernetes containers")
    }

    async fn container_start(&self, connector_id: String) -> () {
        todo!("docker start")
    }

    async fn container_deploy(&self, connector: &Connector) -> Option<OrchestratorContainer> {
        todo!("kubernetes deploy")
    }
}