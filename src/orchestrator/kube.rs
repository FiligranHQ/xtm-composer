use async_trait::async_trait;
use log::info;
use crate::config::settings::{Kube};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};

#[derive(Default)]
pub struct KubeOrchestrator {
    base_uri: String,
}

impl KubeOrchestrator {
    pub fn new(config: &Kube) -> Self {
        let base_uri = config.api.clone();
        Self { base_uri }
    }
}

#[async_trait]
impl Orchestrator for KubeOrchestrator {
    async fn containers(&self) -> Option<Vec<OrchestratorContainer>> {
        info!("{}", self.base_uri);
        Some(Vec::new())
    }
}