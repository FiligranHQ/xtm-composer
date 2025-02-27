mod signals;
mod api;
mod config;
mod orchestrator;

use std::collections::HashMap;
use std::time::Duration;
use crate::api::connector;
use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kube::KubeOrchestrator;
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use env_logger::{Builder, Target};

use log::info;
use log::LevelFilter::Info;
use tokio::time::interval;

const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

async fn orchestrate(settings_data: &Settings, orchestrator: &Box<dyn Orchestrator>) {
    // Get current containers in the orchestrator
    let containers = orchestrator.containers().await.unwrap_or_default();
    let containers_by_id: HashMap<String, OrchestratorContainer> = containers.into_iter()
        .map(|n| (n.extract_opencti_id().clone(), n.clone())).collect();
    // Get the current definition from OpenCTI
    let connectors_response = connector::list(&settings_data).await.data;
    if connectors_response.is_some() {
        let connectors = connectors_response.unwrap().connectors_for_manager.unwrap_or_default();
        // Iter on each definition and check alignment between the status and the container
        for connector in connectors {
            let local_connector = connector.clone();
            info!("CONNECTOR GET {:?} - {:?}", local_connector.id, local_connector.name);
            let assigned_container = containers_by_id.get(local_connector.id.inner());
            if assigned_container.is_none() {
                // Connector is not provisioned, deploy the images
                info!("[X] CONNECTOR IS NOT DEPLOY: {}", local_connector.id.inner());
                orchestrator.container_deploy(&connector).await.unwrap();
            } else {
                // Connector is provisioned
                let assigned = assigned_container.unwrap();
                info!("[V] CONNECTOR IS DEPLOY: {} - {}", local_connector.id.inner(), assigned.state);
                // First check - is version of deployment aligned
                // TODO If not, upgrade
                // Second check - is status aligned
                if !assigned.state.eq("running") {
                    orchestrator.container_start(assigned.id.clone()).await;
                }
                // We need to align the status
                // let connector_status = local_connector.manager_status.unwrap_or_default();
                // match connector_status.as_str() {
                //     "provisioning" => {
                //         let deployed = orchestrator.deploy(&connector).await.unwrap();
                //         Some(deployed)
                //     }
                //     _ => None
                // };
            }
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
                interval.tick().await;
                orchestrate(&settings_data, &orchestrator).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}
