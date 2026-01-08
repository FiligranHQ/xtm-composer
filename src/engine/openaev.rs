use tokio::task::JoinHandle;
use tracing::info;
use crate::api::ComposerApi;
use crate::api::openaev::ApiOpenAEV;
use crate::engine::{alive, orchestration};

pub fn openaev_orchestration() -> JoinHandle<()> {
    info!("Starting OpenAEV connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenAEV::new());
        orchestration(api).await;
    })
}

pub fn openaev_alive() -> JoinHandle<()> {
    info!("Starting OpenAEV Composer ping alive");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenAEV::new());
        alive(api).await;
    })
}

