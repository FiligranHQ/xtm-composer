// TODO Remove macro after implementation
#![allow(unused_variables)]

use async_trait::async_trait;
use log::debug;
use crate::api::connector::{Connector, ConnectorCurrentStatus};
use crate::config::settings::{Kubernetes, Settings};
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
        debug!("Kube base ui: {}", self.base_uri);
        todo!("kubernetes containers")
    }

    async fn container_start(&self, container_id: String) -> () {
        todo!("kubernetes start")
    }

    async fn container_stop(&self, container_id: String) -> () {
        todo!("kubernetes stop")
    }

    async fn container_deploy(&self, settings_data: &Settings, connector: &Connector) -> Option<OrchestratorContainer> {
        todo!("kubernetes deploy")
    }

    async fn container_logs(&self, container_id: String) -> Vec<String> {
        todo!("kubernetes logs")
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus {
        todo!("kubernetes state_converter")
    }
}