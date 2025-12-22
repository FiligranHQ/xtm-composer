use crate::config::settings::Daemon;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

pub mod openaev;
pub mod opencti;
mod decrypt_value;

#[derive(Debug, Clone, Serialize)]
pub struct EnvVariable {
    pub key: String,
    pub value: String,
    pub is_sensitive: bool,
}

#[derive(Debug, Clone)]
pub struct ApiContractConfig {
    pub key: String,
    pub value: String,
    pub is_sensitive: bool,
}

#[derive(Debug, Clone)]
pub struct ApiConnector {
    pub id: String,
    pub name: String,
    pub image: String,
    pub contract_hash: String,
    pub current_status: Option<String>,
    pub requested_status: String,
    pub contract_configuration: Vec<ApiContractConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectorStatus {
    Started,
    Stopped,
}

impl FromStr for ConnectorStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<ConnectorStatus, Self::Err> {
        match input {
            "created" => Ok(ConnectorStatus::Stopped),
            "exited" => Ok(ConnectorStatus::Stopped),
            "started" => Ok(ConnectorStatus::Started),
            "healthy" => Ok(ConnectorStatus::Started),
            "running" => Ok(ConnectorStatus::Started),
            _ => Ok(ConnectorStatus::Stopped),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RequestedStatus {
    Starting,
    Stopping,
}

impl FromStr for RequestedStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<RequestedStatus, Self::Err> {
        match input {
            "starting" => Ok(RequestedStatus::Starting),
            "stopping" => Ok(RequestedStatus::Stopping),
            _ => Ok(RequestedStatus::Stopping),
        }
    }
}

impl ApiConnector {
    pub fn container_name(&self) -> String {
        self.name
            .clone()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .to_lowercase()
    }

    pub fn container_envs(&self) -> Vec<EnvVariable> {
        let settings = crate::settings();
        let mut envs = self
            .contract_configuration
            .iter()
            .map(|config| EnvVariable {
                key: config.key.clone(),
                value: config.value.clone(),
                is_sensitive: config.is_sensitive,
            })
            .collect::<Vec<EnvVariable>>();
        if settings.opencti.enable {
            envs.push(EnvVariable {
                key: "OPENCTI_URL".into(),
                value: settings.opencti.url.clone(),
                is_sensitive: false,
            });
        }
        if settings.openaev.enable {
            envs.push(EnvVariable {
                key: "OPENAEV_URL".into(),
                value: settings.openaev.url.clone(),
                is_sensitive: false,
            });
        }
        envs.push(EnvVariable {
            key: "OPENCTI_CONFIG_HASH".into(),
            value: self.contract_hash.clone(),
            is_sensitive: false,
        });
        envs
    }

    /// Display environment variables with sensitive values masked (if configured)
    pub fn display_env_variables(&self) {
        let settings = crate::settings();

        // Check if display is enabled in configuration
        let should_display = settings
            .manager
            .debug
            .as_ref()
            .map_or(false, |debug| debug.show_env_vars);

        if !should_display {
            return;
        }

        // Check if we should show sensitive values
        let show_sensitive = settings
            .manager
            .debug
            .as_ref()
            .map_or(false, |debug| debug.show_sensitive_env_vars);

        let envs = self.container_envs();

        // Build environment variables map with masked sensitive values
        let env_vars: HashMap<String, String> = envs
            .into_iter()
            .map(|env| {
                let value = if env.is_sensitive && !show_sensitive {
                    "***REDACTED***".to_string()
                } else {
                    env.value
                };
                (env.key, value)
            })
            .collect();

        // Log with structured fields
        info!(
            connector_name = %self.name,
            container_name = %self.container_name(),
            env_vars = ?env_vars,
            "Starting connector"
        );
    }
}

#[async_trait]
pub trait ComposerApi {
    fn daemon(&self) -> &Daemon;

    fn post_logs_schedule(&self) -> Duration;

    async fn version(&self) -> Option<String>;

    async fn ping_alive(&self) -> Option<String>;

    async fn register(&self) -> ();

    async fn connectors(&self) -> Option<Vec<ApiConnector>>;

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector>;

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String>;

    async fn patch_health(
        &self,
        id: String,
        restart_count: u32,
        started_at: String,
        is_in_reboot_loop: bool,
    ) -> Option<String>;
}
