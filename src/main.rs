mod api;
mod config;
mod engine;
mod orchestrator;
mod system;

use crate::config::settings::Settings;
use crate::engine::openbas::{openbas_alive, openbas_orchestration};
use crate::engine::opencti::{opencti_alive, opencti_orchestration};
use futures::future::join_all;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::sync::OnceLock;
use std::{env, fs};
use tokio::task::JoinHandle;
use tracing::{Level, info};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, layer::SubscriberExt};
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_DIRECTORY_LOG: &str = "logs";
const BASE_DIRECTORY_SIZE: usize = 5;
const PREFIX_LOG_NAME: &str = "xtm-composer.log";

// Singleton settings for all application
fn settings() -> &'static Settings {
    static CONFIG: OnceLock<Settings> = OnceLock::new();
    CONFIG.get_or_init(|| Settings::new().unwrap())
}

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

// Init opencti
pub fn verify_opencti_credentials_key() {
    let setting = settings();
    let crendentials_key = &setting.manager.credentials_key;

    // Ensure that the key looks correct
    if !crendentials_key.starts_with("-----BEGIN PRIVATE KEY-----") || !crendentials_key.ends_with("-----END PRIVATE KEY-----") {
        panic!(
            "Invalid private key format"
        );
    }

    // Attempt to create an RsaPrivateKey from PEM data
    match RsaPrivateKey::from_pkcs8_pem(crendentials_key) {
        Ok(..) => {
            info!("Successfully created RsaPrivateKey");
        },
        Err(e) => {
            panic!("Failed to decode private key: {}", e);
        },
    };
}

fn opencti_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.opencti.enable {
        verify_opencti_credentials_key();
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
    let setting = settings();
    if setting.openbas.enable {
        let openbas_alive = openbas_alive();
        orchestrations.push(openbas_alive);
        let openbas_orchestration = openbas_orchestration();
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
