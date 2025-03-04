mod api;
mod config;
mod orchestrator;
mod system;

use crate::api::ComposerApi;
use crate::api::openbas::openbas::ApiOpenBAS;
use crate::api::opencti::opencti::ApiOpenCTI;
use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::portainer::PortainerOrchestrator;
use crate::orchestrator::{Orchestrator, composer};
use crate::system::signals;
use futures::future::join_all;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;
use std::{env, fs};
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{Level, info};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, layer::SubscriberExt};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_DIRECTORY_LOG: &str = "logs";
const BASE_DIRECTORY_SIZE: usize = 5;
const PREFIX_LOG_NAME: &str = "xtm-composer.log";

const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

// Singleton settings for all application
fn settings() -> &'static Settings {
    static CONFIG: OnceLock<Settings> = OnceLock::new();
    CONFIG.get_or_init(|| Settings::new().unwrap())
}

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
                interval.tick().await;
                composer::orchestrate(&orchestrator, &api).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}

fn openbas_orchestration() -> JoinHandle<()> {
    info!("Starting OpenBAS connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenBAS::new());
        orchestration(api).await;
    })
}

fn opencti_orchestration() -> JoinHandle<()> {
    info!("Starting OpenCTI connectors orchestration");
    tokio::spawn(async move {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(ApiOpenCTI::new());
        orchestration(api).await;
    })
}

fn init_logger() -> () {
    let setting = Settings::new().unwrap();
    let current_exe_patch = env::current_exe().unwrap();
    let parent_path = current_exe_patch.parent().unwrap();
    let condition = RollingConditionBasic::new().daily();
    let log_path = parent_path.join(BASE_DIRECTORY_LOG);
    fs::create_dir(log_path.clone()).unwrap_or_default();
    let log_file = log_path.join(PREFIX_LOG_NAME);
    let file_appender =
        BasicRollingFileAppender::new(log_file, condition, BASE_DIRECTORY_SIZE).unwrap();
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    let logger_config = setting.manager.logger;
    let log_level = Level::from_str(logger_config.level.as_str()).unwrap();
    let console_layer = Layer::new()
        .with_writer(std::io::stdout.with_max_level(log_level))
        .pretty();
    let file_layer = Layer::new()
        .with_writer(file_writer.with_max_level(log_level))
        .json();
    Registry::default()
        .with(logger_config.directory.then(|| console_layer))
        .with(logger_config.console.then(|| file_layer))
        .init();
}

#[tokio::main]
async fn main() {
    // Initialize the global logging system
    init_logger();
    // Log the start
    let env = Settings::mode();
    info!(version = VERSION, env, "Starting XTM composer");
    // Start according orchestration threads
    let mut orchestrations = Vec::new();
    let setting = settings();
    if setting.opencti.enable {
        let opencti = opencti_orchestration();
        orchestrations.push(opencti);
    }
    if setting.openbas.enable {
        let openbas = openbas_orchestration();
        orchestrations.push(openbas);
    }
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
