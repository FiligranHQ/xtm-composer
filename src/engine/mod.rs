pub mod openaev;
pub mod opencti;

use crate::api::ComposerApi;
use crate::config::settings::Daemon;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::podman;
use crate::orchestrator::portainer::docker::PortainerDockerOrchestrator;
use crate::orchestrator::swarm::SwarmOrchestrator;
use crate::orchestrator::{Orchestrator, composer};
use crate::settings;
use crate::system::signals;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time::interval;

async fn build_orchestrator(daemon_configuration: &Daemon) -> Box<dyn Orchestrator + Send + Sync> {
    // Delay between two attempts to reach the Docker or Podman API at startup
    let retry_interval = Duration::from_secs(settings().manager.execute_schedule.max(1));
    match daemon_configuration.selector.as_str() {
        "portainer" => match daemon_configuration.portainer.clone() {
            Some(config) => match config.env_type.as_str() {
                "docker" => Box::new(PortainerDockerOrchestrator::new(config)),
                def => panic!("Invalid portainer type configuration: {}", def),
            },
            None => panic!("Missing portainer configuration"),
        },
        "kubernetes" => match daemon_configuration.kubernetes.clone() {
            Some(config) => Box::new(KubeOrchestrator::new(config).await),
            None => panic!("Missing kubernetes configuration"),
        },
        "docker" => Box::new(DockerOrchestrator::new(daemon_configuration, retry_interval).await),
        "podman" => Box::new(podman::connect(daemon_configuration, retry_interval).await),
        "swarm" => match daemon_configuration.swarm.clone() {
            Some(config) => Box::new(SwarmOrchestrator::new(config)),
            None => panic!("Missing swarm configuration"),
        },
        def => panic!("Invalid daemon configuration: {}", def),
    }
}

async fn orchestration(api: Box<dyn ComposerApi + Send + Sync>) {
    let settings = settings();
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            // Get current deployment in target orchestrator
            let orchestrator = build_orchestrator(api.daemon()).await;
            // Created once the orchestrator is ready: ticks missed while it waits for its socket would run back-to-back
            let mut interval = interval(Duration::from_secs(settings.manager.execute_schedule));
            let mut log_ticks = HashMap::new();
            let mut health_tick = Instant::now();
            loop {
                interval.tick().await; // Wait for period
                composer::orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}

pub async fn alive(api: Box<dyn ComposerApi + Send + Sync>) -> JoinHandle<()> {
    let settings = settings();
    let mut interval = interval(Duration::from_secs(settings.manager.ping_alive_schedule));
    tokio::spawn(async move {
        // Start scheduling
        tokio::select! {
            _ = signals::handle_stop_signals() => {}
            _ = async {
                // Infinite retry loop for initial connection
                loop {
                    let version = api.version().await;
                    match version {
                        Some(version) => {
                            // Connection successful - register and start ping loop
                            api.register().await;
                            let mut detected_version: String = version.clone();
                            loop {
                                let ping_response = api.ping_alive().await;
                                match ping_response {
                                    Some(platform_version) => {
                                        // Register when version changes
                                        if platform_version != detected_version {
                                            api.register().await;
                                            detected_version = platform_version;
                                        }
                                    }
                                    _ => {
                                        // Connection lost - break to outer retry loop
                                        break;
                                    }
                                }
                                interval.tick().await;
                            }
                        },
                        None => {
                            // Connection failed - wait and retry
                            interval.tick().await;
                        }
                    }
                }
            } => {
                // This branch will never be reached due to the infinite loop.
            }
        }
    })
}
