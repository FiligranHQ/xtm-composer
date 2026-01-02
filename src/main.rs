mod api;
mod config;
mod engine;
mod orchestrator;
mod prometheus;
mod system;

use crate::config::settings::Settings;
use crate::engine::openbas::{openbas_alive, openbas_orchestration};
use crate::engine::opencti::{opencti_alive, opencti_orchestration};
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
    let setting = &crate::config::settings::SETTINGS;
    let logger_config = &setting.manager.logger;

    // Validate log level
    let log_level = match Level::from_str(&logger_config.level) {
        Ok(level) => level,
        Err(_) => panic!(
            "Invalid log level: '{}'. Valid values are: trace, debug, info, warn, error",
            logger_config.level
        ),
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

// Init opencti
fn opencti_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = &crate::config::settings::SETTINGS;
    if setting.opencti.enable {
        // Initialize private key singleton
        let _ = &crate::config::rsa::PRIVATE_KEY;
        let opencti_alive = opencti_alive();
        orchestrations.push(opencti_alive);
        let opencti_orchestration = opencti_orchestration();
        orchestrations.push(opencti_orchestration);
    } else {
        info!("OpenCTI connectors orchestration disabled");
    }
}

// Init openbas
fn openbas_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = &crate::config::settings::SETTINGS;
    if setting.openbas.enable {
        let openbas_alive = openbas_alive();
        orchestrations.push(openbas_alive);
        let openbas_orchestration = openbas_orchestration();
        orchestrations.push(openbas_orchestration);
    } else {
        info!("OpenBAS connectors orchestration disabled");
    }
}

// Init prometheus metrics server
fn prometheus_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = &crate::config::settings::SETTINGS;
    if let Some(prometheus_config) = &setting.manager.prometheus {
        if prometheus_config.enable {
            let port = prometheus_config.port;
            let handle = tokio::spawn(async move {
                crate::prometheus::start_metrics_server(port).await;
            });
            orchestrations.push(handle);
        }
    }
}

// Main function
#[tokio::main]
async fn main() {
    // Initialize the global logging system
    init_logger();
    // Log the start
    let env = Settings::mode();
    info!(version = VERSION, env, "Starting XTM composer");
    // Start threads
    let mut orchestrations = Vec::new();
    prometheus_orchestrate(&mut orchestrations);
    opencti_orchestrate(&mut orchestrations);
    openbas_orchestrate(&mut orchestrations);
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
