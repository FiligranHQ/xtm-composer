use crate::api::{ApiConnector, ConnectorStatus};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use tracing::error;

pub mod composer;
pub mod docker;
pub mod image;
pub mod kubernetes;
pub mod podman;
pub mod portainer;
pub mod swarm;

/// Product-agnostic label and environment keys tagging the workloads deployed
/// by the composer, whatever their platform (#136).
pub const LABEL_MANAGER: &str = "xtm-manager";
pub const LABEL_WORKLOAD_ID: &str = "xtm-workload-id";
pub const LABEL_PLATFORM: &str = "xtm-platform";
pub const ENV_CONFIG_HASH: &str = "XTM_CONFIG_HASH";

/// Keys used before #136. During the transition they are written alongside the
/// current keys, so that an older composer still recognizes the workloads after
/// a rollback, and read as a fallback, so that the workloads deployed by older
/// versions keep being recognized until they are redeployed.
pub const LEGACY_LABEL_MANAGER: &str = "opencti-manager";
pub const LEGACY_LABEL_WORKLOAD_ID: &str = "opencti-connector-id";
pub const LEGACY_LABEL_PLATFORM: &str = "opencti-platform";
pub const LEGACY_ENV_CONFIG_HASH: &str = "OPENCTI_CONFIG_HASH";

/// Label filters (`key=value`) matching the workloads of a manager under the
/// current and the legacy manager keys. Orchestrators combine multiple label
/// filters with AND, so callers issue one query per filter and merge the
/// results with [`dedup_containers`].
pub fn manager_label_filters(manager_id: &str) -> [String; 2] {
    [
        format!("{LABEL_MANAGER}={manager_id}"),
        format!("{LEGACY_LABEL_MANAGER}={manager_id}"),
    ]
}

/// Drop the containers listed more than once, e.g. a workload carrying both the
/// current and the legacy manager labels, keeping the first occurrence.
pub fn dedup_containers(containers: Vec<OrchestratorContainer>) -> Vec<OrchestratorContainer> {
    let mut seen = HashSet::new();
    containers
        .into_iter()
        .filter(|container| seen.insert(container.id.clone()))
        .collect()
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
        self.workload_id().is_some()
    }

    /// Workload id label, falling back to the legacy key.
    pub fn workload_id(&self) -> Option<&String> {
        self.labels
            .get(LABEL_WORKLOAD_ID)
            .or_else(|| self.labels.get(LEGACY_LABEL_WORKLOAD_ID))
    }

    /// Platform label, falling back to the legacy key.
    pub fn platform(&self) -> Option<&String> {
        self.labels
            .get(LABEL_PLATFORM)
            .or_else(|| self.labels.get(LEGACY_LABEL_PLATFORM))
    }

    pub fn extract_workload_id(&self) -> String {
        self.workload_id().unwrap().clone()
    }

    /// Configuration hash env, read from the legacy key first. Both keys are
    /// written with the same value, but a composer rolled back to an older
    /// version only updates the legacy one when it refreshes a workload (on
    /// Kubernetes, its merge patch leaves the current annotation untouched):
    /// the legacy value is the up-to-date one as long as it is written.
    pub fn extract_config_hash(&self) -> &String {
        self.envs
            .get(LEGACY_ENV_CONFIG_HASH)
            .or_else(|| self.envs.get(ENV_CONFIG_HASH))
            .unwrap()
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

/// Labels of a deployed workload, under both the current and the legacy keys.
pub fn build_labels(manager_id: &str, connector: &ApiConnector) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = HashMap::new();
    for (key, legacy_key, value) in [
        (LABEL_MANAGER, LEGACY_LABEL_MANAGER, manager_id),
        (LABEL_WORKLOAD_ID, LEGACY_LABEL_WORKLOAD_ID, connector.id.as_str()),
        (LABEL_PLATFORM, LEGACY_LABEL_PLATFORM, connector.platform.as_str()),
    ] {
        labels.insert(key.into(), value.to_string());
        labels.insert(legacy_key.into(), value.to_string());
    }
    labels
}

/// Configuration hash entries of a deployed workload, under both the current
/// and the legacy keys.
pub fn config_hash_entries(hash: &str) -> [(String, String); 2] {
    [
        (ENV_CONFIG_HASH.to_string(), hash.to_string()),
        (LEGACY_ENV_CONFIG_HASH.to_string(), hash.to_string()),
    ]
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
            undecryptable_keys: vec![],
        };

        let labels = build_labels("test-manager", &connector);

        assert_eq!(labels.get(LABEL_WORKLOAD_ID), Some(&connector.id));
        assert_eq!(labels.get(LABEL_PLATFORM), Some(&connector.platform));
        assert_eq!(labels.get(LABEL_MANAGER), Some(&"test-manager".to_string()));
    }

    /// The legacy labels are still written so that an older composer keeps
    /// recognizing the workloads after a rollback.
    #[test]
    fn labels_keep_the_legacy_keys() {
        let connector = ApiConnector {
            id: "connector-1".to_string(),
            platform: "openaev".to_string(),
            name: String::new(),
            image: String::new(),
            contract_hash: String::new(),
            current_status: None,
            requested_status: String::new(),
            contract_configuration: vec![],
            undecryptable_keys: vec![],
        };

        let labels = build_labels("test-manager", &connector);

        assert_eq!(labels.len(), 6);
        assert_eq!(labels.get("opencti-connector-id"), Some(&connector.id));
        assert_eq!(labels.get("opencti-platform"), Some(&connector.platform));
        assert_eq!(labels.get("opencti-manager"), Some(&"test-manager".to_string()));
    }

    #[test]
    fn config_hash_entries_cover_current_and_legacy_keys() {
        let entries: HashMap<String, String> = config_hash_entries("h1").into_iter().collect();
        assert_eq!(entries.get("XTM_CONFIG_HASH").map(String::as_str), Some("h1"));
        assert_eq!(entries.get("OPENCTI_CONFIG_HASH").map(String::as_str), Some("h1"));
    }

    fn container_with(labels: &[(&str, &str)], envs: &[(&str, &str)]) -> OrchestratorContainer {
        OrchestratorContainer {
            id: "container-id".to_string(),
            name: "container-name".to_string(),
            state: "running".to_string(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            envs: envs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            restart_count: 0,
            started_at: None,
        }
    }

    #[test]
    fn accessors_read_current_keys() {
        let container = container_with(
            &[(LABEL_WORKLOAD_ID, "A"), (LABEL_PLATFORM, "openaev")],
            &[(ENV_CONFIG_HASH, "h1")],
        );
        assert!(container.is_managed());
        assert_eq!(container.extract_workload_id(), "A");
        assert_eq!(container.platform().map(String::as_str), Some("openaev"));
        assert_eq!(container.extract_config_hash(), "h1");
    }

    #[test]
    fn accessors_fall_back_to_legacy_keys() {
        let container = container_with(
            &[(LEGACY_LABEL_WORKLOAD_ID, "A"), (LEGACY_LABEL_PLATFORM, "opencti")],
            &[(LEGACY_ENV_CONFIG_HASH, "h1")],
        );
        assert!(container.is_managed());
        assert_eq!(container.extract_workload_id(), "A");
        assert_eq!(container.platform().map(String::as_str), Some("opencti"));
        assert_eq!(container.extract_config_hash(), "h1");
    }

    #[test]
    fn accessors_prefer_current_labels_over_legacy_ones() {
        let container = container_with(
            &[
                (LABEL_WORKLOAD_ID, "new"),
                (LEGACY_LABEL_WORKLOAD_ID, "old"),
                (LABEL_PLATFORM, "openaev"),
                (LEGACY_LABEL_PLATFORM, "opencti"),
            ],
            &[],
        );
        assert_eq!(container.extract_workload_id(), "new");
        assert_eq!(container.platform().map(String::as_str), Some("openaev"));
    }

    /// A composer rolled back to an older version only updates the legacy hash.
    #[test]
    fn config_hash_prefers_the_legacy_key() {
        let container = container_with(
            &[],
            &[(ENV_CONFIG_HASH, "stale-hash"), (LEGACY_ENV_CONFIG_HASH, "refreshed-hash")],
        );
        assert_eq!(container.extract_config_hash(), "refreshed-hash");
    }

    #[test]
    fn container_without_workload_id_is_not_managed() {
        let container = container_with(&[(LABEL_MANAGER, "m"), (LEGACY_LABEL_MANAGER, "m")], &[]);
        assert!(!container.is_managed());
        assert!(container.platform().is_none());
    }

    #[test]
    fn manager_label_filters_cover_current_and_legacy_keys() {
        assert_eq!(
            manager_label_filters("m"),
            ["xtm-manager=m".to_string(), "opencti-manager=m".to_string()]
        );
    }

    #[test]
    fn dedup_containers_keeps_first_occurrence_by_id() {
        let mut first = container_with(&[], &[]);
        first.id = "a".to_string();
        first.name = "first".to_string();
        let mut duplicate = first.clone();
        duplicate.name = "duplicate".to_string();
        let mut other = first.clone();
        other.id = "b".to_string();

        let containers = dedup_containers(vec![first, duplicate, other]);

        let ids: Vec<&str> = containers.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(containers[0].name, "first");
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
    fn selector_immutability_conflict_matches_kubernetes_message() {
        // Real message emitted by the Kubernetes API when the immutable
        // spec.selector no longer matches the new template labels.
        let mismatch = "Deployment.apps \"nmap--the-network-mapper-83257074\" is invalid: \
            spec.template.metadata.labels: Invalid value: \
            map[string]string{\"app\":\"connector\"}: \
            `selector` does not match template `labels`";
        assert!(KubeOrchestrator::is_selector_immutability_conflict(
            mismatch
        ));

        let immutable = "Deployment.apps \"x\" is invalid: spec.selector: \
            Invalid value: v1.LabelSelector{...}: field is immutable";
        assert!(KubeOrchestrator::is_selector_immutability_conflict(
            immutable
        ));
    }

    #[test]
    fn selector_immutability_conflict_ignores_other_422_errors() {
        // Unrelated validation failures must not trigger the self-heal
        // deletion of a possibly-healthy deployment.
        let bad_env = "Deployment.apps \"x\" is invalid: \
            spec.template.spec.containers[0].env[0].name: Invalid value: \
            \"BAD NAME\": a valid environment variable name must consist of...";
        assert!(!KubeOrchestrator::is_selector_immutability_conflict(
            bad_env
        ));

        let bad_quantity = "Deployment.apps \"x\" is invalid: \
            spec.template.spec.containers[0].resources.limits[memory]: \
            Invalid value: \"10Gx\": unable to parse quantity's suffix";
        assert!(!KubeOrchestrator::is_selector_immutability_conflict(
            bad_quantity
        ));
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