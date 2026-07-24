use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;
use crate::api::ComposerApi;
use crate::api::opencti::ApiOpenCTI;
use crate::engine::{alive, orchestration};

/// Create a shared readiness channel for OpenCTI alive/orchestration coordination.
pub fn opencti_ready() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

pub fn opencti_alive(ready: watch::Sender<bool>) -> JoinHandle<()> {
    info!("Starting OpenCTI Composer ping alive");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new());
        alive(api, ready).await;
    })
}

pub fn opencti_orchestration(ready: watch::Receiver<bool>) -> JoinHandle<()> {
    info!("Starting OpenCTI connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new());
        orchestration(api, ready).await;
    })
}