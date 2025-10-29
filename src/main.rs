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
use tracing::{Level, info, warn};
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

// Singleton RSA private key for all application
pub fn private_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| load_and_verify_credentials_key())
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

// Load and verify RSA private key from configuration
pub fn load_and_verify_credentials_key() -> RsaPrivateKey {
    let setting = settings();
    
    // Priority: file > environment variable
    let key_content = if let Some(filepath) = &setting.manager.credentials_key_filepath {
        // Warning if both are set
        if setting.manager.credentials_key.is_some() {
            warn!("Both credentials_key and credentials_key_filepath are set. Using filepath (priority).");
        }
        
        // Read key from file
        match fs::read_to_string(filepath) {
            Ok(content) => content,
            Err(e) => panic!("Failed to read credentials key file '{}': {}", filepath, e)
        }
    } else if let Some(key) = &setting.manager.credentials_key {
        // Use environment variable or config value
        key.clone()
    } else {
        panic!(
            "No credentials key provided! Set either 'manager.credentials_key' or 'manager.credentials_key_filepath' in configuration."
        );
    };
    
    // Validate key format (trim to handle trailing whitespace)
    // Check for presence of RSA PRIVATE KEY markers for PKCS#8 format
    let trimmed_content = key_content.trim();
    if !trimmed_content.contains("BEGIN PRIVATE KEY") || !trimmed_content.contains("END PRIVATE KEY") {
        panic!("Invalid private key format. Expected PKCS#8 PEM format with 'BEGIN PRIVATE KEY' and 'END PRIVATE KEY' markers.");
    }
    
    // Parse and validate RSA private key using PKCS#8 format
    match RsaPrivateKey::from_pkcs8_pem(&key_content) {
        Ok(key) => {
            info!("Successfully loaded RSA private key (PKCS#8 format)");
            key
        },
        Err(e) => {
            panic!("Failed to decode RSA private key: {}", e);
        }
    }
}

fn opencti_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.opencti.enable {
        // Initialize private key singleton
        let _ = private_key();
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
    // Start registry cache cleanup task
    orchestrator::start_registry_cache_cleanup();
    info!("Started registry authentication cache cleanup task");
    // Start orchestration threads
    let mut orchestrations = Vec::new();
    opencti_orchestrate(&mut orchestrations);
    openbas_orchestrate(&mut orchestrations);
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
