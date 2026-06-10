use crate::api::{ApiConnector, ConnectorStatus};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::error;

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

pub fn ensure_proxy_ca_file(connector: &ApiConnector) -> Option<String> {
    let cert_content = connector.proxy_ca_bundle()?;

    let base_dir: PathBuf = std::env::temp_dir().join("xtm-composer-proxy-ca");
    if let Err(err) = fs::create_dir_all(&base_dir) {
        error!(
            path = %base_dir.display(),
            error = err.to_string(),
            "Unable to create temporary directory for proxy CA bundle"
        );
        return None;
    }

    let normalized_id: String = connector
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let target_path = base_dir.join(format!(
        "{}-{}-proxy-ca.crt",
        connector.platform, normalized_id
    ));
    if let Err(err) = fs::write(&target_path, &cert_content) {
        error!(
            path = %target_path.display(),
            error = err.to_string(),
            "Unable to write proxy CA bundle to temporary file"
        );
        return None;
    }

    Some(target_path.to_string_lossy().to_string())
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
    use crate::orchestrator::kubernetes::KubeOrchestrator;

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

    #[test]
    fn refresh_patch_strips_selector_from_deployment_spec() {
        // refresh() strips spec.selector from the merge patch so that
        // the immutable field is never sent to Kubernetes.
        use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
        use std::collections::BTreeMap;

        let deployment = Deployment {
            spec: Some(DeploymentSpec {
                replicas: Some(1),
                selector: LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "opencti-connector-id".to_string(),
                        "abc-123".to_string(),
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut patch_value = serde_json::to_value(&deployment).unwrap();
        if let Some(spec) = patch_value.pointer_mut("/spec") {
            spec.as_object_mut().unwrap().remove("selector");
        }

        let spec = patch_value.get("spec").expect("spec must exist");
        assert!(
            spec.get("selector").is_none(),
            "selector should be stripped from the patch: {spec}"
        );
        assert_eq!(
            spec.get("replicas").and_then(|v| v.as_i64()),
            Some(1),
            "other spec fields must survive"
        );
    }

    #[test]
    fn deploy_payload_includes_all_labels_in_selector() {
        // deploy() sends the full Deployment including the selector with
        // all labels so Kubernetes can match pods precisely.
        use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
        use std::collections::BTreeMap;

        let labels: BTreeMap<String, String> = BTreeMap::from([
            ("opencti-manager".to_string(), "test-manager".to_string()),
            ("opencti-connector-id".to_string(), "connector-42".to_string()),
            ("opencti-platform".to_string(), "opencti".to_string()),
        ]);
        let deployment = Deployment {
            spec: Some(DeploymentSpec {
                selector: LabelSelector {
                    match_labels: Some(labels.clone()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_value(&deployment).unwrap();
        let match_labels = json
            .pointer("/spec/selector/matchLabels")
            .expect("matchLabels must be present");
        assert_eq!(
            match_labels.get("opencti-connector-id").and_then(|v| v.as_str()),
            Some("connector-42"),
            "selector must contain the connector-id label"
        );
        assert_eq!(
            match_labels.get("opencti-manager").and_then(|v| v.as_str()),
            Some("test-manager"),
            "selector must contain the manager label"
        );
        assert_eq!(
            match_labels.get("opencti-platform").and_then(|v| v.as_str()),
            Some("opencti"),
            "selector must contain the platform label"
        );
    }

    #[test]
    fn build_refresh_patch_strips_selector() {
        // This test calls KubeOrchestrator::build_refresh_patch directly.
        use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
        use std::collections::BTreeMap;

        let deployment = Deployment {
            spec: Some(DeploymentSpec {
                replicas: Some(2),
                selector: LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "opencti-connector-id".to_string(),
                        "abc-123".to_string(),
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        let patch = KubeOrchestrator::build_refresh_patch(&deployment);

        let spec = patch.get("spec").expect("spec must be present");
        assert!(
            spec.get("selector").is_none(),
            "build_refresh_patch() must strip spec.selector — got: {spec}"
        );
        assert_eq!(
            spec.get("replicas").and_then(|v: &serde_json::Value| v.as_i64()),
            Some(2),
            "other spec fields must survive"
        );
    }
}