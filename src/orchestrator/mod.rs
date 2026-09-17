use crate::api::{ApiWorkload, WorkloadStatus};
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

/// Current, product-agnostic label and environment keys used to tag and
/// identify the workloads managed by the composer.
pub const LABEL_MANAGER: &str = "xtm-manager";
pub const LABEL_WORKLOAD_ID: &str = "xtm-workload-id";
pub const LABEL_PLATFORM: &str = "xtm-platform";
pub const ENV_CONFIG_HASH: &str = "XTM_CONFIG_HASH";

/// Legacy keys used before #136. Still read (never written) so that workloads
/// deployed by older composer versions keep being recognized during the
/// transition period.
pub const LEGACY_LABEL_MANAGER: &str = "opencti-manager";
pub const LEGACY_LABEL_WORKLOAD_ID: &str = "opencti-connector-id";
pub const LEGACY_LABEL_PLATFORM: &str = "opencti-platform";
pub const LEGACY_ENV_CONFIG_HASH: &str = "OPENCTI_CONFIG_HASH";

/// Label filter values (`key=value`) matching a manager id for both the current
/// and legacy manager label keys. Callers issue one query per value and merge
/// the results so listing stays backward compatible.
pub fn manager_label_filters(manager_id: &str) -> [String; 2] {
    [
        format!("{LABEL_MANAGER}={manager_id}"),
        format!("{LEGACY_LABEL_MANAGER}={manager_id}"),
    ]
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

    /// Workload id read from the current label, falling back to the legacy one
    /// for workloads deployed before #136.
    pub fn workload_id(&self) -> Option<&String> {
        self.labels
            .get(LABEL_WORKLOAD_ID)
            .or_else(|| self.labels.get(LEGACY_LABEL_WORKLOAD_ID))
    }

    pub fn extract_workload_id(&self) -> String {
        self.workload_id().unwrap().clone()
    }

    /// Platform label read from the current key, falling back to the legacy one.
    pub fn platform(&self) -> Option<&String> {
        self.labels
            .get(LABEL_PLATFORM)
            .or_else(|| self.labels.get(LEGACY_LABEL_PLATFORM))
    }

    pub fn extract_config_hash(&self) -> &String {
        self.envs
            .get(ENV_CONFIG_HASH)
            .or_else(|| self.envs.get(LEGACY_ENV_CONFIG_HASH))
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

pub fn build_labels(manager_id: &str, workload: &ApiWorkload) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = HashMap::new();
    labels.insert(LABEL_MANAGER.into(), manager_id.to_string());
    labels.insert(LABEL_WORKLOAD_ID.into(), workload.id.clone());
    labels.insert(LABEL_PLATFORM.into(), workload.platform.clone());
    labels
}

pub fn ensure_proxy_ca_file(workload: &ApiWorkload) -> Option<String> {
    let cert_content = workload.proxy_ca_bundle()?;

    let base_dir: PathBuf = std::env::temp_dir().join("xtm-composer-proxy-ca");
    if let Err(err) = fs::create_dir_all(&base_dir) {
        error!(
            path = %base_dir.display(),
            error = err.to_string(),
            "Unable to create temporary directory for proxy CA bundle"
        );
        return None;
    }

    let normalized_id: String = workload
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let target_path = base_dir.join(format!(
        "{}-{}-proxy-ca.crt",
        workload.platform, normalized_id
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
    fn labels(&self, workload: &ApiWorkload) -> HashMap<String, String> {
        build_labels(&crate::settings().manager.id, workload)
    }

    async fn get(&self, workload: &ApiWorkload) -> Option<OrchestratorContainer>;

    async fn list(&self) -> Vec<OrchestratorContainer>;

    async fn start(&self, container: &OrchestratorContainer, workload: &ApiWorkload) -> ();

    async fn stop(&self, container: &OrchestratorContainer, workload: &ApiWorkload) -> ();

    async fn remove(&self, container: &OrchestratorContainer) -> ();

    async fn refresh(&self, workload: &ApiWorkload) -> Option<OrchestratorContainer>;

    async fn deploy(&self, workload: &ApiWorkload) -> Option<OrchestratorContainer>;

    async fn logs(
        &self,
        container: &OrchestratorContainer,
        workload: &ApiWorkload,
    ) -> Option<Vec<String>>;

    fn state_converter(&self, container: &OrchestratorContainer) -> WorkloadStatus;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::kubernetes::KubeOrchestrator;

    #[test]
    fn labels_include_platform_discriminator() {
        let workload = ApiWorkload {
            id: "workload-1".to_string(),
            platform: "opencti".to_string(),
            name: String::new(),
            image: String::new(),
            contract_hash: String::new(),
            current_status: None,
            requested_status: String::new(),
            contract_configuration: vec![],
        };

        let labels = build_labels("test-manager", &workload);

        assert_eq!(labels.get(LABEL_WORKLOAD_ID), Some(&workload.id));
        assert_eq!(labels.get(LABEL_PLATFORM), Some(&workload.platform));
        assert_eq!(labels.get(LABEL_MANAGER), Some(&"test-manager".to_string()));
    }

    fn container_with(
        labels: &[(&str, &str)],
        envs: &[(&str, &str)],
    ) -> OrchestratorContainer {
        OrchestratorContainer {
            id: "id".to_string(),
            name: "name".to_string(),
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
    fn accessors_read_current_labels_and_envs() {
        let container = container_with(
            &[(LABEL_WORKLOAD_ID, "workload-1"), (LABEL_PLATFORM, "openaev")],
            &[(ENV_CONFIG_HASH, "hash-1")],
        );

        assert!(container.is_managed());
        assert_eq!(container.extract_workload_id(), "workload-1");
        assert_eq!(container.platform().map(String::as_str), Some("openaev"));
        assert_eq!(container.extract_config_hash(), "hash-1");
    }

    #[test]
    fn accessors_fall_back_to_legacy_labels_and_envs() {
        // Workloads deployed before #136 only carry the legacy keys and must
        // still be recognized so they are not orphaned after an upgrade.
        let container = container_with(
            &[
                (LEGACY_LABEL_WORKLOAD_ID, "workload-1"),
                (LEGACY_LABEL_PLATFORM, "opencti"),
            ],
            &[(LEGACY_ENV_CONFIG_HASH, "hash-1")],
        );

        assert!(container.is_managed());
        assert_eq!(container.extract_workload_id(), "workload-1");
        assert_eq!(container.platform().map(String::as_str), Some("opencti"));
        assert_eq!(container.extract_config_hash(), "hash-1");
    }

    #[test]
    fn current_labels_take_precedence_over_legacy() {
        let container = container_with(
            &[
                (LABEL_WORKLOAD_ID, "new"),
                (LEGACY_LABEL_WORKLOAD_ID, "old"),
            ],
            &[(ENV_CONFIG_HASH, "new"), (LEGACY_ENV_CONFIG_HASH, "old")],
        );

        assert_eq!(container.extract_workload_id(), "new");
        assert_eq!(container.extract_config_hash(), "new");
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
                        LABEL_WORKLOAD_ID.to_string(),
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
            (LABEL_MANAGER.to_string(), "test-manager".to_string()),
            (LABEL_WORKLOAD_ID.to_string(), "workload-42".to_string()),
            (LABEL_PLATFORM.to_string(), "opencti".to_string()),
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
            match_labels.get(LABEL_WORKLOAD_ID).and_then(|v| v.as_str()),
            Some("workload-42"),
            "selector must contain the workload-id label"
        );
        assert_eq!(
            match_labels.get(LABEL_MANAGER).and_then(|v| v.as_str()),
            Some("test-manager"),
            "selector must contain the manager label"
        );
        assert_eq!(
            match_labels.get(LABEL_PLATFORM).and_then(|v| v.as_str()),
            Some("opencti"),
            "selector must contain the platform label"
        );
    }

    #[test]
    fn selector_immutability_conflict_matches_kubernetes_message() {
        // Real message emitted by the Kubernetes API when the immutable
        // spec.selector no longer matches the new template labels.
        let mismatch = "Deployment.apps \"nmap--the-network-mapper-83257074\" is invalid: \
            spec.template.metadata.labels: Invalid value: \
            map[string]string{\"app\":\"workload\"}: \
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
                        LABEL_WORKLOAD_ID.to_string(),
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