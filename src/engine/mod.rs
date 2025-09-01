pub mod openbas;
pub mod opencti;

use crate::api::ComposerApi;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::portainer::docker::PortainerDockerOrchestrator;
use crate::orchestrator::{Orchestrator, composer};
use crate::settings;
use crate::system::signals;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time::interval;

async fn orchestration(api: Box<dyn ComposerApi + Send + Sync>) {
    let settings = settings();
    // Get current deployment in target orchestrator
    let daemon_configuration = api.daemon();
    let orchestrator: Box<dyn Orchestrator + Send + Sync> =
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
            "docker" => Box::new(DockerOrchestrator::new()),
            def => panic!("Invalid daemon configuration: {}", def),
        };
    // Init scheduler interval
    let mut interval = interval(Duration::from_secs(settings.manager.execute_schedule));
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            let mut tick = Instant::now();
            let mut health_tick = Instant::now();
            loop {
                interval.tick().await; // Wait for period
                composer::orchestrate(&mut tick, &mut health_tick, &orchestrator, &api).await;
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
                // Get the api version
                let version = api.version().await;
                match version {
                    Some(version) => {
                        // Register the manager with contracts align with api version
                        api.register().await;
                        let mut detected_version: String = version.clone();
                        loop {
                            let ping_response = api.ping_alive().await;
                            match ping_response {
                                Some(platform_version) => {
                                    // Register the manager at start or when version change
                                    if platform_version != detected_version {
                                        api.register().await;
                                        detected_version = platform_version;
                                    }
                                }
                                _ => {
                                    // Error already handle in upper level
                                }
                            }
                            interval.tick().await; // Wait for period
                        }
                    },
                    _ => {
                        // Error already handle in upper level
                    }
                }

            } => {
                // This branch will never be reached due to the infinite loop.
            }
        }
    })
}
