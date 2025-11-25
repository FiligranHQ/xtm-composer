use xtm_composer::{settings, private_key};
use xtm_composer::config::settings::Settings;
use futures::future::join_all;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::{env, fs};
use tokio::task::JoinHandle;
use tracing::{Level, info};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, layer::SubscriberExt};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_DIRECTORY_LOG: &str = "logs";
const BASE_DIRECTORY_SIZE: usize = 5;
const PREFIX_LOG_NAME: &str = "xtm-composer.log";

// Global init logger
fn init_logger() {
    let setting = Settings::new().unwrap();
    let logger_config = &setting.manager.logger;

    // Validate log level
    let log_level = match Level::from_str(&logger_config.level) {
        Ok(level) => level,
        Err(_) => panic!(
            "Invalid log level: '{}'. Valid values are: trace, debug, info, warn, error",
            logger_config.level
        )
    };

    // Validate log format
    if logger_config.format != "json" && logger_config.format != "pretty" {
        panic!(
            "Invalid log format: '{}'. Valid values are: json, pretty",
            logger_config.format
        );
    }

    let current_exe_patch = env::current_exe().unwrap();
    let parent_path = current_exe_patch.parent().unwrap();
    let condition = RollingConditionBasic::new().daily();
    let log_path = parent_path.join(BASE_DIRECTORY_LOG);
    fs::create_dir(log_path.clone()).unwrap_or_default();
    let log_file = log_path.join(PREFIX_LOG_NAME);
    let file_appender =
        BasicRollingFileAppender::new(log_file, condition, BASE_DIRECTORY_SIZE).unwrap();
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);

    if logger_config.format == "json" {
        let console_layer = Layer::new()
            .with_writer(std::io::stdout.with_max_level(log_level))
            .json();
        let file_layer = Layer::new()
            .with_writer(file_writer.with_max_level(log_level))
            .json();
        Registry::default()
            .with(logger_config.directory.then(|| console_layer))
            .with(logger_config.console.then(|| file_layer))
            .init();
    } else {
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
}

fn opencti_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.opencti.enable {
        // Initialize private key singleton
        let _ = private_key();
        let opencti_alive = xtm_composer::engine::opencti::opencti_alive();
        orchestrations.push(opencti_alive);
        let opencti_orchestration = xtm_composer::engine::opencti::opencti_orchestration();
        orchestrations.push(opencti_orchestration);
    } else {
        info!("OpenCTI connectors orchestration disabled");
    }
}

// Init openbas
fn openbas_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.openbas.enable {
        let openbas_alive = xtm_composer::engine::openbas::openbas_alive();
        orchestrations.push(openbas_alive);
        let openbas_orchestration = xtm_composer::engine::openbas::openbas_orchestration();
        orchestrations.push(openbas_orchestration);
    } else {
        info!("OpenBAS connectors orchestration disabled");
    }
}

#[tokio::main]
async fn main() {
    // Initialize the global logging system
    init_logger();
    // Log the start
    let env = Settings::mode();
    info!(version = VERSION, env, "Starting XTM composer");
    // Start orchestration threads
    let mut orchestrations = Vec::new();
    opencti_orchestrate(&mut orchestrations);
    openbas_orchestrate(&mut orchestrations);
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
