use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;
use crate::api::ComposerApi;
use crate::api::openaev::ApiOpenAEV;
use crate::engine::{alive, orchestration};

/// Create a shared readiness channel for OpenAEV alive/orchestration coordination.
pub fn openaev_ready() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

pub fn openaev_orchestration(ready: watch::Receiver<bool>) -> JoinHandle<()> {
    info!("Starting OpenAEV connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenAEV::new());
        orchestration(api, ready).await;
    })
}

pub fn openaev_alive(ready: watch::Sender<bool>) -> JoinHandle<()> {
    info!("Starting OpenAEV Composer ping alive");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenAEV::new());
        alive(api, ready).await;
    })
}

