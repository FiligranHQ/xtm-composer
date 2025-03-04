use crate::api::opencti::connector::{ConnectorCurrentStatus, EnvVariable};
use crate::config::settings::{Daemon, Settings};
use async_trait::async_trait;

pub mod opencti;
pub mod openbas;

#[derive(Debug, Clone)]
pub struct ApiContractConfig {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ApiConnector {
    pub id: String,
    pub name: String,
    pub image: String,
    pub contract_hash: String,
    pub current_status: Option<String>,
    pub requested_status: String,
    pub contract_configuration: Vec<ApiContractConfig>
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

    pub fn container_envs(
        &self,
        settings: &Settings,
    ) -> Vec<EnvVariable> {
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
            value: settings.opencti.url.clone(),
        });
        envs.push(EnvVariable {
            key: "OPENCTI_CONFIG_HASH".into(),
            value: self.contract_hash.clone(),
        });
        envs
    }
}

#[async_trait]
pub trait ComposerApi {
    fn daemon(&self) -> &Daemon;

    async fn register(&self, settings: &Settings) -> Option<String>;

    async fn connectors(&self, settings: &Settings) -> Option<Vec<ApiConnector>>;

    async fn patch_status(
        &self,
        connector_id: String,
        status: ConnectorCurrentStatus,
    ) -> Option<ApiConnector>;

    async fn patch_logs(&self, connector_id: String, logs: Vec<String>)
    -> Option<ApiConnector>;
}
