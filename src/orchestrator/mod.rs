use crate::api::{ApiConnector, ConnectorStatus};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;

pub mod composer;
pub mod docker;
pub mod image;
pub mod kubernetes;
pub mod portainer;
pub mod swarm;

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

pub fn build_labels(manager_id: &str, connector: &ApiConnector) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = HashMap::new();
    labels.insert("opencti-manager".into(), manager_id.to_string());
    labels.insert("opencti-connector-id".into(), connector.id.clone());
    labels.insert("opencti-platform".into(), connector.platform.clone());
    labels
}

#[async_trait]
pub trait Orchestrator {
    fn labels(&self, connector: &ApiConnector) -> HashMap<String, String> {
        build_labels(&crate::settings().manager.id, connector)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_include_platform_discriminator() {
        let connector = ApiConnector {
            id: "connector-1".to_string(),
            platform: "opencti".to_string(),
            name: String::new(),
            image: String::new(),
            contract_hash: String::new(),
            current_status: None,
            requested_status: String::new(),
            contract_configuration: vec![],
        };

        let labels = build_labels("test-manager", &connector);

        assert_eq!(labels.get("opencti-connector-id"), Some(&connector.id));
        assert_eq!(labels.get("opencti-platform"), Some(&connector.platform));
        assert_eq!(labels.get("opencti-manager"), Some(&"test-manager".to_string()));
    }
}
