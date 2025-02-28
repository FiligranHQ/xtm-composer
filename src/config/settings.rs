use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

const ENV_PRODUCTION: &str = "production";

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct Manager {
    pub id: String,
    pub name: String,
    pub daemon: String,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct OpenCTI {
    pub url: String,
    pub token: String,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
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

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct Kubernetes {
    pub api: String,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct Settings {
    pub debug: bool,
    pub manager: Manager,
    pub opencti: OpenCTI,
    pub portainer: Portainer,
    pub kubernetes: Kubernetes,
}

impl Settings {
    pub fn mode() -> String {
        env::var("env").unwrap_or_else(|_| ENV_PRODUCTION.into())
    }

    pub fn new() -> Result<Self, ConfigError> {
        let run_mode = Self::mode();
        let config = Config::builder().add_source(Environment::with_prefix("opencti"));
        config
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            .build()?
            .try_deserialize()
    }
}
