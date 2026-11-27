use crate::api::{ApiWorkload, WorkloadStatus};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
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
    /// Set when the orchestrator cannot pull the image of the running deployment
    #[serde(skip)]
    pub image_pull_error: Option<ImagePullError>,
    /// Digest of the image the workload actually runs (`sha256:...`), so a moving
    /// tag (`:rolling`, `:latest`) can be verified; None when the engine does not tell
    #[serde(skip)]
    pub image_digest: Option<String>,
    /// How the previous run of the workload container ended (OOMKilled, exit code,
    /// time), so a failure stays diagnosable after a restart; None when the engine
    /// does not tell or the container never ended
    #[serde(skip)]
    pub last_termination: Option<ContainerTermination>,
}

/// How a container run ended, as reported by the engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerTermination {
    /// Reason given by the engine (`OOMKilled`, `Error`, `Completed`, ...)
    pub reason: Option<String>,
    pub exit_code: Option<i64>,
    /// RFC 3339 time the run ended
    pub finished_at: Option<String>,
}

impl ContainerTermination {
    /// One log line telling how the previous container run ended.
    pub fn log_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(reason) = &self.reason {
            parts.push(format!("reason {reason}"));
        }
        if let Some(exit_code) = self.exit_code {
            parts.push(format!("exit code {exit_code}"));
        }
        if let Some(finished_at) = &self.finished_at {
            parts.push(format!("at {finished_at}"));
        }
        format!(
            "[XTM Composer] The previous container ended: {}",
            if parts.is_empty() { "unknown".to_string() } else { parts.join(", ") }
        )
    }
}

/// Termination of a Docker-compatible container that is no longer running, read
/// from its inspected state. Docker resets the exit code when the container starts
/// again, so the termination of a running container is not available (None).
pub fn docker_termination(
    running: bool,
    exit_code: Option<i64>,
    oom_killed: Option<bool>,
    error: Option<&str>,
    finished_at: Option<&str>,
) -> Option<ContainerTermination> {
    // Docker writes 0001-01-01T00:00:00Z for a container that never ended
    let finished_at = finished_at.filter(|time| !time.is_empty() && !time.starts_with("0001-"));
    if running || finished_at.is_none() {
        return None;
    }
    let reason = if oom_killed == Some(true) {
        "OOMKilled".to_string()
    } else if let Some(error) = error.filter(|error| !error.is_empty()) {
        error.to_string()
    } else if exit_code.unwrap_or(0) != 0 {
        "Error".to_string()
    } else {
        "Completed".to_string()
    };
    Some(ContainerTermination {
        reason: Some(reason),
        exit_code,
        finished_at: finished_at.map(str::to_string),
    })
}

/// Lines kept from the container run before the current one, per workload.
pub const PREVIOUS_LOG_LINES: usize = 50;

/// Latest log lines of the current and the previous container run of each
/// workload, so the platform still shows the end of a run after the container or
/// the pod is replaced (the platform keeps only the latest pushed snapshot).
/// Bounded: one entry per workload, at most [`PREVIOUS_LOG_LINES`] lines per run.
#[derive(Default)]
pub struct LogHistory {
    workloads: Mutex<HashMap<String, WorkloadLogRuns>>,
}

#[derive(Default)]
struct WorkloadLogRuns {
    current_run: String,
    current: Vec<String>,
    previous: Vec<String>,
}

fn tail(lines: &[String], count: usize) -> Vec<String> {
    lines[lines.len().saturating_sub(count)..].to_vec()
}

impl LogHistory {
    /// Records the latest lines of the run `run_id` (a container id, a pod name) of
    /// a workload and returns the last lines of the run before it, empty when the
    /// workload had no other run since the composer started.
    pub fn record(&self, workload_id: &str, run_id: &str, lines: &[String]) -> Vec<String> {
        let mut workloads = self
            .workloads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let runs = workloads.entry(workload_id.to_string()).or_default();
        if runs.current_run != run_id {
            if !runs.current.is_empty() {
                runs.previous = std::mem::take(&mut runs.current);
            }
            runs.current_run = run_id.to_string();
        }
        runs.current = tail(lines, PREVIOUS_LOG_LINES);
        runs.previous.clone()
    }

    /// Drops the history of a removed workload.
    pub fn forget(&self, workload_id: &str) {
        self.workloads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(workload_id);
    }
}

/// The logs pushed to the platform: the last lines of the previous `run` (container
/// or pod), when there are any, under a marker, then the current run's lines.
pub fn with_previous_logs(previous: Vec<String>, run: &str, current: Vec<String>) -> Vec<String> {
    if previous.is_empty() {
        return current;
    }
    let mut logs = Vec::with_capacity(previous.len() + current.len() + 2);
    logs.push(format!("[XTM Composer] Last lines of the previous {run}:"));
    logs.extend(previous);
    logs.push(format!("[XTM Composer] Logs of the current {run}:"));
    logs.extend(current);
    logs
}

/// Digest (`sha256:...`) of an image reference or image id: the part after `@` of
/// `repo@sha256:...` or `docker-pullable://repo@sha256:...`, or a bare `sha256:...` id.
/// None for an empty reference or a reference without a digest (`repo:tag`).
pub fn image_digest_of(reference: &str) -> Option<String> {
    let reference = reference.trim();
    let digest = match reference.rsplit_once('@') {
        Some((_, digest)) => digest,
        None => reference,
    };
    match digest.split_once(':') {
        Some((algorithm, hex))
            if !algorithm.is_empty()
                && !algorithm.contains('/')
                && hex.len() >= 32
                && hex.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            Some(digest.to_string())
        }
        _ => None,
    }
}

/// Running image digest of a Docker-compatible engine: the first repository digest of
/// the inspected image, else the image id the container runs.
pub fn running_image_digest(repo_digests: &[String], image_id: Option<&str>) -> Option<String> {
    repo_digests
        .iter()
        .find_map(|reference| image_digest_of(reference))
        .or_else(|| image_id.and_then(image_digest_of))
}

/// An image that cannot be pulled, with the error of the engine
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePullError {
    pub image: String,
    pub error: String,
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
pub fn build_labels(manager_id: &str, workload: &ApiWorkload) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = HashMap::new();
    for (key, legacy_key, value) in [
        (LABEL_MANAGER, LEGACY_LABEL_MANAGER, manager_id),
        (LABEL_WORKLOAD_ID, LEGACY_LABEL_WORKLOAD_ID, workload.id.as_str()),
        (LABEL_PLATFORM, LEGACY_LABEL_PLATFORM, workload.platform.as_str()),
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

    /// The name the engine knows the workload by. Engines that cap names at an
    /// RFC 1123 label (Kubernetes, Swarm) return [`ApiWorkload::dns_label_name`].
    fn workload_name(&self, workload: &ApiWorkload) -> String {
        workload.container_name()
    }

    async fn get(&self, workload: &ApiWorkload) -> Option<OrchestratorContainer>;

    async fn list(&self) -> Vec<OrchestratorContainer>;

    async fn start(&self, container: &OrchestratorContainer, workload: &ApiWorkload) -> ();

    async fn stop(&self, container: &OrchestratorContainer, workload: &ApiWorkload) -> ();

    async fn remove(&self, container: &OrchestratorContainer) -> ();

    /// Pull the image of the workload, called before deploy and refresh so that a
    /// missing image never replaces a running deployment. Orchestrators whose nodes
    /// pull the image themselves keep the default.
    async fn pull(&self, _workload: &ApiWorkload) -> Result<(), ImagePullError> {
        Ok(())
    }

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

    fn lines(prefix: &str, count: usize) -> Vec<String> {
        (0..count).map(|index| format!("{prefix} {index}")).collect()
    }

    #[test]
    fn log_history_returns_the_last_lines_of_the_replaced_run() {
        let history = LogHistory::default();
        assert!(history.record("w1", "pod-a", &lines("a", 3)).is_empty());
        // Same run read again: nothing replaced yet
        assert!(history.record("w1", "pod-a", &lines("a", 4)).is_empty());
        // The pod was replaced: the last read of pod-a comes back
        assert_eq!(history.record("w1", "pod-b", &lines("b", 2)), lines("a", 4));
        // And keeps coming back while pod-b runs
        assert_eq!(history.record("w1", "pod-b", &lines("b", 5)), lines("a", 4));
        // Another workload has its own history
        assert!(history.record("w2", "pod-c", &lines("c", 1)).is_empty());
    }

    #[test]
    fn log_history_is_bounded_per_run() {
        let history = LogHistory::default();
        history.record("w1", "pod-a", &lines("a", 500));
        let previous = history.record("w1", "pod-b", &[]);
        assert_eq!(previous.len(), PREVIOUS_LOG_LINES);
        assert_eq!(previous.last().unwrap(), "a 499");
    }

    #[test]
    fn log_history_keeps_the_last_run_with_lines_when_the_new_one_is_empty() {
        let history = LogHistory::default();
        history.record("w1", "pod-a", &lines("a", 2));
        history.record("w1", "pod-b", &[]);
        // pod-b never logged: pod-a stays the previous run
        assert_eq!(history.record("w1", "pod-c", &lines("c", 1)), lines("a", 2));
    }

    #[test]
    fn log_history_forgets_a_removed_workload() {
        let history = LogHistory::default();
        history.record("w1", "pod-a", &lines("a", 2));
        history.forget("w1");
        assert!(history.record("w1", "pod-b", &lines("b", 1)).is_empty());
    }

    #[test]
    fn previous_logs_come_first_under_markers() {
        assert_eq!(with_previous_logs(vec![], "pod", lines("b", 1)), lines("b", 1));
        assert_eq!(
            with_previous_logs(lines("a", 1), "container", lines("b", 1)),
            vec![
                "[XTM Composer] Last lines of the previous container:".to_string(),
                "a 0".to_string(),
                "[XTM Composer] Logs of the current container:".to_string(),
                "b 0".to_string(),
            ]
        );
    }

    #[test]
    fn docker_termination_reads_an_oom_kill() {
        let termination =
            docker_termination(false, Some(137), Some(true), None, Some("2026-11-26T10:00:00Z"))
                .unwrap();
        assert_eq!(termination.reason.as_deref(), Some("OOMKilled"));
        assert_eq!(termination.exit_code, Some(137));
        assert_eq!(
            termination.log_line(),
            "[XTM Composer] The previous container ended: reason OOMKilled, exit code 137, at 2026-11-26T10:00:00Z"
        );
    }

    #[test]
    fn docker_termination_reads_an_error_exit() {
        let termination =
            docker_termination(false, Some(1), Some(false), Some(""), Some("2026-11-26T10:00:00Z"))
                .unwrap();
        assert_eq!(termination.reason.as_deref(), Some("Error"));
    }

    #[test]
    fn docker_termination_is_unknown_while_running_or_never_ended() {
        assert!(docker_termination(true, Some(0), Some(false), None, Some("2026-11-26T10:00:00Z")).is_none());
        assert!(docker_termination(false, Some(0), Some(false), None, Some("0001-01-01T00:00:00Z")).is_none());
        assert!(docker_termination(false, None, None, None, None).is_none());
    }

    #[test]
    fn labels_include_platform_discriminator() {
        let workload = ApiWorkload {
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

        let labels = build_labels("test-manager", &workload);

        assert_eq!(labels.get(LABEL_WORKLOAD_ID), Some(&workload.id));
        assert_eq!(labels.get(LABEL_PLATFORM), Some(&workload.platform));
        assert_eq!(labels.get(LABEL_MANAGER), Some(&"test-manager".to_string()));
    }

    /// The legacy labels are still written so that an older composer keeps
    /// recognizing the workloads after a rollback.
    #[test]
    fn labels_keep_the_legacy_keys() {
        let workload = ApiWorkload {
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

        let labels = build_labels("test-manager", &workload);

        assert_eq!(labels.len(), 6);
        assert_eq!(labels.get("opencti-connector-id"), Some(&workload.id));
        assert_eq!(labels.get("opencti-platform"), Some(&workload.platform));
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
            image_pull_error: None,
            image_digest: None,
            last_termination: None,
        }
    }

    const DIGEST: &str =
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn image_digest_is_read_from_references_and_ids() {
        assert_eq!(
            image_digest_of(&format!("docker-pullable://openaev/injector-nmap@{}", DIGEST)),
            Some(DIGEST.to_string())
        );
        assert_eq!(
            image_digest_of(&format!("docker.io/openaev/injector-nmap@{}", DIGEST)),
            Some(DIGEST.to_string())
        );
        assert_eq!(image_digest_of(DIGEST), Some(DIGEST.to_string()));
        assert_eq!(image_digest_of(""), None);
        assert_eq!(image_digest_of("openaev/injector-nmap:rolling"), None);
        assert_eq!(image_digest_of("localhost:5000/injector-nmap"), None);
    }

    #[test]
    fn running_image_digest_prefers_the_repository_digest() {
        let other = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        assert_eq!(
            running_image_digest(
                &[format!("openaev/injector-nmap@{}", DIGEST)],
                Some(other)
            ),
            Some(DIGEST.to_string())
        );
        assert_eq!(running_image_digest(&[], Some(other)), Some(other.to_string()));
        assert_eq!(running_image_digest(&[], None), None);
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