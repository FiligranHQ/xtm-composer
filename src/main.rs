mod api;
mod config;
mod orchestrator;
mod system;

use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::orchestrator::{composer, Orchestrator};
use env_logger::{Builder, Target};
use std::time::Duration;

use crate::system::signals;
use log::LevelFilter::Info;
use tokio::time::interval;

const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

#[tokio::main]
async fn main() {
    // Init logger
    Builder::new()
        .filter_level(Info)
        .target(Target::Stdout)
        .init();
    // Build settings
    let settings = Settings::new();
    let settings_data = settings.unwrap();
    // Get OpenCTI managed connectors
    let daemon_type = &settings_data.manager.daemon;
    // Get current deployment in target orchestrator
    let orchestrator: Box<dyn Orchestrator> = match daemon_type.as_str() {
        "portainer" => Box::new(PortainerOrchestrator::new(&settings_data.portainer)),
        "kubernetes" => Box::new(KubeOrchestrator::new(&settings_data.kubernetes).await),
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
                composer::orchestrate(&settings_data, &orchestrator).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}
