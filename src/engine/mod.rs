pub mod opencti;
pub mod openbas;

use std::time::{Duration};
use tokio::task::JoinHandle;
use tokio::time::interval;
use crate::{ALIVE_TIMER, SCHEDULER_TIMER};
use crate::api::ComposerApi;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::{composer, Orchestrator};
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::system::signals;

async fn orchestration(api: Box<dyn ComposerApi + Send + Sync>) -> () {
    // Register the manager in OpenCTI
    api.register().await;
    // Get current deployment in target orchestrator
    let daemon_configuration = api.daemon();
    let orchestrator: Box<dyn Orchestrator + Send + Sync> =
        match daemon_configuration.selector.as_str() {
            "portainer" => match daemon_configuration.portainer.clone() {
                Some(config) => Box::new(PortainerOrchestrator::new(config)),
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
    let mut interval = interval(Duration::from_secs(SCHEDULER_TIMER));
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            loop {
                interval.tick().await; // Wait for period
                composer::orchestrate(&orchestrator, &api).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}

pub async fn alive(api: Box<dyn ComposerApi + Send + Sync>) -> JoinHandle<()>  {
    let mut interval = interval(Duration::from_secs(ALIVE_TIMER));
    tokio::spawn(async move {
        // Start scheduling
        tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            loop {
                interval.tick().await; // Wait for period
                api.ping_alive().await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
    })
}