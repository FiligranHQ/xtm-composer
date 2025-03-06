use tokio::task::JoinHandle;
use tracing::info;
use crate::api::ComposerApi;
use crate::api::openbas::ApiOpenBAS;
use crate::engine::{alive, orchestration};

pub fn openbas_orchestration() -> JoinHandle<()> {
    info!("Starting OpenBAS connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenBAS::new());
        orchestration(api).await;
    })
}

pub fn openbas_alive() -> JoinHandle<()> {
    info!("Starting OpenBAS Composer ping alive");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenBAS::new());
        alive(api).await;
    })
}

