mod api;
mod config;
mod orchestrator;
mod system;

use crate::api::openbas::openbas::ApiOpenBAS;
use crate::api::opencti::opencti::ApiOpenCTI;
use crate::api::ComposerApi;
use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::orchestrator::{composer, Orchestrator};
use crate::system::signals;
use env_logger::{Builder, Target};
use futures::future::join_all;
use log::info;
use log::LevelFilter::Info;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::interval;

const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

async fn orchestration(settings: Settings, api: Box<dyn ComposerApi + Send + Sync>) -> () {
    // Register the manager in OpenCTI
    api.register(&settings).await;
    // Get OpenCTI managed connectors
    let daemon_type = &settings.manager.daemon;
    // Get current deployment in target orchestrator
    let orchestrator:Box<dyn Orchestrator + Send + Sync> = match daemon_type.as_str() {
        "portainer" => Box::new(PortainerOrchestrator::new(&settings.portainer)),
        "kubernetes" => Box::new(KubeOrchestrator::new(&settings.kubernetes).await),
        "docker" => Box::new(DockerOrchestrator::new()),
        def => panic!("Invalid daemon configuration: {}", def),
    };
    // Init scheduler interval
    let mut interval = interval(Duration::from_secs(SCHEDULER_TIMER));
    // Start scheduling
    tokio::select! {
            _ = signals::handle_stop_signals() => {}
            _ = async {
                loop {
                    interval.tick().await;
                    composer::orchestrate(&settings, &orchestrator, &api).await;
                }
            } => {
                // This branch will never be reached due to the infinite loop.
            }
        }
}

fn openbas_orchestration(settings: Settings) -> JoinHandle<()> {
    info!("[/] Starting OpenBAS connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenBAS::new(&settings));
        orchestration(settings, api).await;
    })
}

fn opencti_orchestration(settings: Settings) -> JoinHandle<()> {
    info!("[/] Starting OpenCTI connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new(&settings));
        orchestration(settings, api).await;
    })
}

#[tokio::main]
async fn main() {
    // Init logger
    Builder::new()
        .filter_level(Info)
        .target(Target::Stdout)
        .init();
    // Register the manager in OpenCTI
    let opencti_setting = Settings::new().unwrap();
    let mut orchestrations = Vec::new();
    if opencti_setting.opencti.enable {
        let opencti = opencti_orchestration(opencti_setting);
        orchestrations.push(opencti);
    }
    // Register the manager in OpenBAS
    let openbas_setting = Settings::new().unwrap();
    if openbas_setting.openbas.enable {
        let openbas = openbas_orchestration(openbas_setting);
        orchestrations.push(openbas);
    }
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
