use crate::config::settings::Daemon;
use crate::config::secret_string::SecretString;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

pub mod openbas;
pub mod opencti;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContractsManifest {
    name: String,
    contracts: Value,
}

/// Environment variable value that can be either public or secret
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum EnvValue {
    Public(String),
    Secret(SecretString),
}

impl EnvValue {
    /// Get the raw string value (for Docker/K8s deployment)
    pub fn as_str(&self) -> &str {
        match self {
            EnvValue::Public(s) => s,
            EnvValue::Secret(secret) => secret.expose_secret(),
        }
    }
    
    /// Check if this value is sensitive
    pub fn is_sensitive(&self) -> bool {
        matches!(self, EnvValue::Secret(_))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvVariable {
    pub key: String,
    pub value: EnvValue,
}

#[derive(Debug, Clone)]
pub struct ApiContractConfig {
    pub key: String,
    pub value: EnvValue,
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
            })
            .collect::<Vec<EnvVariable>>();
        envs.push(EnvVariable {
            key: "OPENCTI_URL".into(),
            value: EnvValue::Public(settings.opencti.url.clone()),
        });
        envs.push(EnvVariable {
            key: "OPENCTI_CONFIG_HASH".into(),
            value: EnvValue::Public(self.contract_hash.clone()),
        });
        envs
    }

    /// Display environment variables with sensitive values masked (if configured)
    pub fn display_env_variables(&self) {
        let settings = crate::settings();
        
        // Check if display is enabled in configuration
        if !settings.manager.debug.as_ref().map_or(false, |d| d.show_env_vars) {
            return;
        }
        
        // Check if we should show sensitive values
        let show_sensitive = settings.manager.debug
            .as_ref()
            .map_or(false, |d| d.show_sensitive_env_vars);
        
        let envs = self.container_envs();
        
        // Build environment variables map with automatic masking via SecretString
        let env_vars: HashMap<String, String> = envs
            .into_iter()
            .map(|env| {
                let value = match &env.value {
                    EnvValue::Public(v) => v.clone(),
                    EnvValue::Secret(s) => {
                        if show_sensitive {
                            s.expose_secret().to_string()
                        } else {
                            format!("{:?}", s) // Automatically "***REDACTED***"
                        }
                    }
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

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<cynic::Id>;

    async fn patch_health(&self, id: String, restart_count: u32, started_at: String, is_in_reboot_loop: bool) -> Option<cynic::Id>;
}
