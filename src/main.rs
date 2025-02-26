mod signals;
mod api;
mod config;
mod orchestrator;

use std::time::Duration;
use crate::api::connector;
use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kube::KubeOrchestrator;
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::orchestrator::Orchestrator;
use env_logger::{Builder, Target};

use log::info;
use log::LevelFilter::Info;
use tokio::time::interval;

const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

async fn orchestrate(settings_data: &Settings, orchestrator: &Box<dyn Orchestrator>) {
    let containers = orchestrator.containers().await;
    for container in containers {
        info!("CONTAINER GET {:?} - {:?}", container.id, container.image);
        info!("CONTAINER LABELS {:?}", container.is_managed());
    }

    let connectors_response = connector::list(&settings_data).await.data;
    match connectors_response {
        Some(connector::GetConnectors { connectors_for_manager: Some(connectors)}) => {
            for connector in connectors {
                info!("CONNECTOR GET {:?} - {:?} - {:?}", connector.connector_type, connector.active, connector.connector_state)
            }
        }
        _ => {
            info!("No connectors found");
        }
    }
}

#[tokio::main]
async fn main() {
    // Init logger
    Builder::new().filter_level(Info).target(Target::Stdout).init();
    // Build settings
    let settings = Settings::new();
    let settings_data = settings.unwrap();
    // Get OpenCTI managed connectors
    let daemon_type = &settings_data.manager.daemon;
    // Get current deployment in target orchestrator
    let orchestrator: Box<dyn Orchestrator> = match daemon_type.as_str() {
        "portainer" => Box::new(PortainerOrchestrator::new(&settings_data.portainer)),
        "kubernetes" => Box::new(KubeOrchestrator::new(&settings_data.kube)),
        "docker" => Box::new(DockerOrchestrator::new()),
        def => panic!("Invalid daemon configuration: {}", def)
    };
    // Init scheduler interval
    let mut interval = interval(Duration::from_secs(SCHEDULER_TIMER));
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            loop {
                info!("Timer tick");
                interval.tick().await;
                orchestrate(&settings_data, &orchestrator).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}
