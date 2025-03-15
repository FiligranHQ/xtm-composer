use config::{Config, ConfigError, Environment, File};
use k8s_openapi::api::apps::v1::Deployment;
use serde::Deserialize;
use std::env;

const ENV_PRODUCTION: &str = "production";

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Logger {
    pub level: String,
    pub directory: bool,
    pub console: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Manager {
    pub id: String,
    pub name: String,
    pub logger: Logger,
    pub execute_schedule: u64,
    pub ping_alive_schedule: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Daemon {
    pub selector: String,
    pub portainer: Option<Portainer>,
    pub kubernetes: Option<Kubernetes>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct OpenCTI {
    pub enable: bool,
    pub url: String,
    pub token: String,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub logs_schedule: u64,
    pub contracts: Vec<String>,
    pub daemon: Daemon,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct OpenBAS {
    pub enable: bool,
    pub url: String,
    pub token: String,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub logs_schedule: u64,
    pub contracts: Vec<String>,
    pub daemon: Daemon,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Portainer {
    pub api: String,
    pub api_key: String,
    pub env_id: String,
    pub env_type: String,
    pub api_version: String,
    pub stack: Option<String>,
    pub network_mode: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Kubernetes {
    pub base_deployment: Option<Deployment>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    pub manager: Manager,
    pub opencti: OpenCTI,
    pub openbas: OpenBAS,
}

impl Settings {
    pub fn mode() -> String {
        env::var("COMPOSER_ENV").unwrap_or_else(|_| ENV_PRODUCTION.into())
    }

    pub fn new() -> Result<Self, ConfigError> {
        let run_mode = Self::mode();
        let config_builder = Config::builder();
        config_builder
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            .add_source(
                Environment::default()
                    .try_parsing(true)
                    .separator("_")
                    .list_separator(","),
            )
            .build()?
            .try_deserialize()
    }
}
