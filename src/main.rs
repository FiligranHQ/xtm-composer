mod api;
mod config;
mod orchestrator;
mod system;
mod engine;

use crate::config::settings::Settings;
use futures::future::join_all;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::str::FromStr;
use std::sync::OnceLock;
use std::{env, fs};
use tracing::{Level, info};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, layer::SubscriberExt};
use crate::engine::openbas::{openbas_alive, openbas_orchestration};
use crate::engine::opencti::{opencti_alive, opencti_orchestration};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_DIRECTORY_LOG: &str = "logs";
const BASE_DIRECTORY_SIZE: usize = 5;
const PREFIX_LOG_NAME: &str = "xtm-composer.log";

const ALIVE_TIMER: u64 = 60; // 1 minute scheduling
const SCHEDULER_TIMER: u64 = 5; // 5 seconds scheduling

// Singleton settings for all application
fn settings() -> &'static Settings {
    static CONFIG: OnceLock<Settings> = OnceLock::new();
    CONFIG.get_or_init(|| Settings::new().unwrap())
}

// Global init logger
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
        let opencti_orchestration = opencti_orchestration();
        orchestrations.push(opencti_orchestration);
        let opencti_alive = opencti_alive();
        orchestrations.push(opencti_alive);
    }
    if setting.openbas.enable {
        let openbas_orchestration = openbas_orchestration();
        orchestrations.push(openbas_orchestration);
        let openbas_alive = openbas_alive();
        orchestrations.push(openbas_alive);
    }
    // Wait for threads to terminate
    join_all(orchestrations).await;
}
