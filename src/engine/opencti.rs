use tokio::task::JoinHandle;
use tracing::info;
use crate::api::ComposerApi;
use crate::api::opencti::ApiOpenCTI;
use crate::engine::{alive, orchestration};

pub fn opencti_alive() -> JoinHandle<()> {
    info!("Starting OpenCTI Composer ping alive");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new());
        alive(api).await;
    })
}

pub fn opencti_orchestration() -> JoinHandle<()> {
    info!("Starting OpenCTI connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new());
        orchestration(api).await;
    })
}