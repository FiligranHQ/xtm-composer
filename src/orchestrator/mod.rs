use crate::api::{ApiConnector, ConnectorStatus};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::debug;

pub mod composer;
pub mod docker;
pub mod kubernetes;
pub mod portainer;
pub mod registry_cache;
pub mod registry_resolver;

/// Start periodic registry cache cleanup
pub fn start_registry_cache_cleanup() {
    tokio::spawn(async {
        let mut interval = interval(TokioDuration::from_secs(10 * 60)); // Every 10 minutes
        loop {
            interval.tick().await;
            debug!("Running registry cache cleanup");
            registry_cache::REGISTRY_AUTH_CACHE.cleanup_expired().await;
        }
    });
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all(deserialize = "PascalCase"))]
pub struct OrchestratorContainer {
    pub id: String,
    pub name: String,
    pub state: String,
    pub labels: HashMap<String, String>,
    pub envs: HashMap<String, String>,
    pub restart_count: u32,
    pub started_at: Option<String>,
}

impl OrchestratorContainer {
    pub fn is_managed(&self) -> bool {
        self.labels.contains_key("opencti-connector-id")
    }

    pub fn extract_opencti_id(&self) -> String {
        self.labels.get("opencti-connector-id").unwrap().clone()
    }

    pub fn extract_opencti_hash(&self) -> &String {
        self.envs.get("OPENCTI_CONFIG_HASH").unwrap()
    }

    pub fn is_in_reboot_loop(&self) -> bool {
        if self.restart_count > 3 {
            if let Some(started_at_str) = &self.started_at {
                if let Ok(started_at) = DateTime::parse_from_rfc3339(started_at_str) {
                    let uptime = Utc::now() - started_at.with_timezone(&Utc);
                    return uptime < Duration::minutes(5);
                }
            }
        }
        false
    }
}

#[async_trait]
pub trait Orchestrator {
    fn labels(&self, connector: &ApiConnector) -> HashMap<String, String> {
        let settings = crate::settings();
        let mut labels: HashMap<String, String> = HashMap::new();
        labels.insert("opencti-manager".into(), settings.manager.id.clone());
        labels.insert("opencti-connector-id".into(), connector.id.clone());
        labels
    }

    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

    async fn list(&self) -> Vec<OrchestratorContainer>;

    async fn start(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> ();

    async fn stop(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> ();

    async fn remove(&self, container: &OrchestratorContainer) -> ();

    async fn refresh(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

    async fn deploy(&self, connector: &ApiConnector) -> Option<OrchestratorContainer>;

    async fn logs(
        &self,
        container: &OrchestratorContainer,
        connector: &ApiConnector,
    ) -> Option<Vec<String>>;

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorStatus;
}
