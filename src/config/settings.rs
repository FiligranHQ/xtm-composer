use config::{Config, ConfigError, Environment, File};
use k8s_openapi::api::apps::v1::Deployment;
use serde::Deserialize;
use std::env;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const ENV_PRODUCTION: &str = "production";

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Logger {
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
    pub directory: bool,
    pub console: bool,
}

fn default_log_format() -> String {
    "json".to_string()
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Debug {
    #[serde(default)]
    pub show_env_vars: bool,
    #[serde(default)]
    pub show_sensitive_env_vars: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Manager {
    pub id: String,
    pub name: String,
    pub logger: Logger,
    pub execute_schedule: u64,
    pub ping_alive_schedule: u64,
    pub credentials_key: Option<String>,
    pub credentials_key_filepath: Option<String>,
    pub debug: Option<Debug>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Daemon {
    pub selector: String,
    pub portainer: Option<Portainer>,
    pub kubernetes: Option<Kubernetes>,
    pub docker: Option<Docker>,
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
    pub base_deployment_json: Option<String>,
    pub image_pull_policy: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Docker {
    pub network_mode: Option<String>,
    pub extra_hosts: Option<Vec<String>>,
    pub dns: Option<Vec<String>>,
    pub dns_search: Option<Vec<String>>,
    pub privileged: Option<bool>,
    pub cap_add: Option<Vec<String>>,
    pub cap_drop: Option<Vec<String>>,
    pub security_opt: Option<Vec<String>>,
    pub userns_mode: Option<String>,
    pub pid_mode: Option<String>,
    pub ipc_mode: Option<String>,
    pub uts_mode: Option<String>,
    pub runtime: Option<String>,
    pub shm_size: Option<i64>,
    pub sysctls: Option<std::collections::HashMap<String, String>>,
    pub ulimits: Option<Vec<std::collections::HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    pub manager: Manager,
    pub opencti: OpenCTI,
    pub openbas: OpenBAS,
    #[serde(skip)]
    pub proxy_cache: Arc<RwLock<ProxyCache>>,
}

#[derive(Debug, Clone)]
pub struct ProxyCache {
    pub configuration: Option<crate::api::opencti::connector::get_proxy_config::ProxyConfiguration>,
    pub last_fetched: Option<Instant>,
    pub ttl: Duration,
}

impl Default for ProxyCache {
    fn default() -> Self {
        ProxyCache {
            configuration: None,
            last_fetched: None,
            ttl: Duration::from_secs(600), // Default 10 minutes TTL
        }
    }
}

impl Settings {
    pub fn mode() -> String {
        env::var("COMPOSER_ENV").unwrap_or_else(|_| ENV_PRODUCTION.into())
    }

    pub fn new() -> Result<Self, ConfigError> {
        let run_mode = Self::mode();
        let config_builder = Config::builder();
        let mut settings: Settings = config_builder
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            .add_source(Environment::default().try_parsing(true).separator("__"))
            .build()?
            .try_deserialize()?;
        
        // Initialize the proxy cache with configurable TTL
        let cache_ttl = env::var("OPENCTI__PROXY_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(600); // Default 10 minutes
        
        settings.proxy_cache = Arc::new(RwLock::new(ProxyCache {
            configuration: None,
            last_fetched: None,
            ttl: Duration::from_secs(cache_ttl),
        }));
        
        Ok(settings)
    }
    
    pub fn is_proxy_cache_valid(&self) -> bool {
        let cache = self.proxy_cache.read().unwrap();
        if let Some(last_fetched) = cache.last_fetched {
            last_fetched.elapsed() < cache.ttl
        } else {
            false
        }
    }
    
    pub fn update_proxy_cache(&self, config: crate::api::opencti::connector::get_proxy_config::ProxyConfiguration) {
        let mut cache = self.proxy_cache.write().unwrap();
        cache.configuration = Some(config);
        cache.last_fetched = Some(Instant::now());
    }
    
    pub fn get_cached_proxy_config(&self) -> Option<crate::api::opencti::connector::get_proxy_config::ProxyConfiguration> {
        let cache = self.proxy_cache.read().unwrap();
        cache.configuration.clone()
    }
}
