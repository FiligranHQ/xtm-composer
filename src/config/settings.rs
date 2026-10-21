use config::{Config, ConfigError, Environment, File};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::ResourceRequirements;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use std::collections::BTreeMap;
use std::env;

const ENV_PRODUCTION: &str = "production";

fn default_https_proxy_reject_unauthorized() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProxyCaRaw {
    List(Vec<String>),
    String(String),
    IndexedMap(BTreeMap<String, String>),
}

fn deserialize_https_proxy_ca<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<ProxyCaRaw>::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(None);
    };

    let entries = match raw {
        ProxyCaRaw::List(list) => list,
        ProxyCaRaw::String(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with('[') {
                serde_json::from_str::<Vec<String>>(trimmed)
                    .map_err(|e| de::Error::custom(format!("invalid https_proxy_ca JSON array: {e}")))?
            } else {
                vec![s]
            }
        }
        ProxyCaRaw::IndexedMap(map) => {
            let mut indexed: Vec<(usize, String)> = Vec::new();
            let mut non_indexed: Vec<(String, String)> = Vec::new();
            for (k, v) in map {
                if let Ok(idx) = k.parse::<usize>() {
                    indexed.push((idx, v));
                } else {
                    non_indexed.push((k, v));
                }
            }
            indexed.sort_by_key(|(idx, _)| *idx);
            non_indexed.sort_by(|a, b| a.0.cmp(&b.0));

            indexed
                .into_iter()
                .map(|(_, v)| v)
                .chain(non_indexed.into_iter().map(|(_, v)| v))
                .collect()
        }
    };

    if entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some(entries))
    }
}

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
pub struct Registry {
    pub server: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Daemon {
    pub selector: String,
    pub registry: Option<Registry>,
    pub portainer: Option<Portainer>,
    pub kubernetes: Option<Kubernetes>,
    pub docker: Option<Docker>,
    pub swarm: Option<Swarm>,
    /// Podman serves the Docker-compatible API: it takes the same options as `docker`.
    pub podman: Option<Docker>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct OpenCTI {
    pub enable: bool,
    pub url: String,
    pub token: String,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy: Option<String>,
    #[serde(default, deserialize_with = "deserialize_https_proxy_ca")]
    pub https_proxy_ca: Option<Vec<String>>,
    #[serde(default = "default_https_proxy_reject_unauthorized")]
    pub https_proxy_reject_unauthorized: bool,
    pub logs_schedule: u64,
    pub request_timeout: u64,
    pub connect_timeout: u64,
    pub daemon: Daemon,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct OpenAEV {
    pub enable: bool,
    pub url: String,
    pub token: String,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy: Option<String>,
    #[serde(default, deserialize_with = "deserialize_https_proxy_ca")]
    pub https_proxy_ca: Option<Vec<String>>,
    #[serde(default = "default_https_proxy_reject_unauthorized")]
    pub https_proxy_reject_unauthorized: bool,
    pub logs_schedule: u64,
    pub request_timeout: u64,
    pub connect_timeout: u64,
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
    pub image_resources: Option<ResourceRequirements>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[allow(unused)]
pub struct Docker {
    /// API socket, as a path or a `unix://` URL. Defaults to `/var/run/docker.sock` for
    /// Docker and to the auto-detected rootless or rootful socket for Podman.
    pub socket_path: Option<String>,
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
pub struct SwarmResources {
    pub cpu_limit: Option<i64>,
    pub memory_limit: Option<i64>,
    pub cpu_reservation: Option<i64>,
    pub memory_reservation: Option<i64>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Swarm {
    pub network: Option<String>,
    pub extra_hosts: Option<Vec<String>>,
    pub dns: Option<Vec<String>>,
    pub dns_search: Option<Vec<String>>,
    pub cap_add: Option<Vec<String>>,
    pub cap_drop: Option<Vec<String>>,
    pub sysctls: Option<std::collections::HashMap<String, String>>,
    pub ulimits: Option<Vec<std::collections::HashMap<String, serde_json::Value>>>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub read_only: Option<bool>,
    pub init: Option<bool>,
    pub stop_grace_period: Option<i64>,
    pub resources: Option<SwarmResources>,
    pub placement_constraints: Option<Vec<String>>,
    pub placement_preferences: Option<Vec<String>>,
    pub restart_condition: Option<String>,
    pub restart_delay: Option<i64>,
    pub restart_max_attempts: Option<i64>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    pub manager: Manager,
    pub opencti: OpenCTI,
    pub openaev: OpenAEV,
}

impl Settings {
    pub fn mode() -> String {
        env::var("COMPOSER_ENV").unwrap_or_else(|_| ENV_PRODUCTION.into())
    }

    pub fn new() -> Result<Self, ConfigError> {
        let run_mode = Self::mode();
        let config_builder = Config::builder();
        config_builder
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            .add_source(Self::environment())
            .build()?
            .try_deserialize()
    }

    fn environment() -> Environment {
        Environment::default().try_parsing(true).separator("__")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct ProxyCaOnly {
        #[serde(default, deserialize_with = "deserialize_https_proxy_ca")]
        https_proxy_ca: Option<Vec<String>>,
    }

    #[test]
    fn deserialize_https_proxy_ca_from_sequence() {
        let input = r#"https_proxy_ca = ["/ca/a.pem", "/ca/b.pem"]"#;
        let cfg: ProxyCaOnly = config::Config::builder()
            .add_source(config::File::from_str(input, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            cfg.https_proxy_ca,
            Some(vec!["/ca/a.pem".to_string(), "/ca/b.pem".to_string()])
        );
    }

    #[test]
    fn deserialize_https_proxy_ca_from_json_array_string() {
        let input = r#"https_proxy_ca = "[\"/ca/a.pem\",\"/ca/b.pem\"]""#;
        let cfg: ProxyCaOnly = config::Config::builder()
            .add_source(config::File::from_str(input, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            cfg.https_proxy_ca,
            Some(vec!["/ca/a.pem".to_string(), "/ca/b.pem".to_string()])
        );
    }

    fn settings_with_env(env: &[(&str, &str)]) -> Settings {
        let env_map: config::Map<String, String> = env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::builder()
            .add_source(File::from_str(
                include_str!("../../config/default.yaml"),
                config::FileFormat::Yaml,
            ))
            .add_source(Settings::environment().source(Some(env_map)))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn default_configuration_has_no_podman_block() {
        let settings = settings_with_env(&[]);
        assert!(settings.opencti.daemon.podman.is_none());
        assert!(settings.openaev.daemon.podman.is_none());
    }

    #[test]
    fn podman_daemon_is_configured_from_environment_for_each_platform() {
        let settings = settings_with_env(&[
            ("OPENCTI__DAEMON__SELECTOR", "podman"),
            ("OPENCTI__DAEMON__PODMAN__SOCKET_PATH", "/run/podman/podman.sock"),
            ("OPENCTI__DAEMON__PODMAN__NETWORK_MODE", "opencti_default"),
            ("OPENCTI__DAEMON__PODMAN__PRIVILEGED", "false"),
            ("OPENAEV__DAEMON__SELECTOR", "podman"),
            ("OPENAEV__DAEMON__PODMAN__SOCKET_PATH", "unix:///run/user/1000/podman/podman.sock"),
        ]);

        let opencti = &settings.opencti.daemon;
        assert_eq!(opencti.selector, "podman");
        let podman = opencti.podman.as_ref().expect("opencti podman block");
        assert_eq!(podman.socket_path.as_deref(), Some("/run/podman/podman.sock"));
        assert_eq!(podman.network_mode.as_deref(), Some("opencti_default"));
        assert_eq!(podman.privileged, Some(false));

        let openaev = &settings.openaev.daemon;
        assert_eq!(openaev.selector, "podman");
        let podman = openaev.podman.as_ref().expect("openaev podman block");
        assert_eq!(
            podman.socket_path.as_deref(),
            Some("unix:///run/user/1000/podman/podman.sock")
        );
        assert!(podman.network_mode.is_none());
    }

    #[test]
    fn podman_daemon_accepts_the_docker_container_options() {
        let input = r#"
            selector: podman
            podman:
              network_mode: host
              extra_hosts:
                - "host.containers.internal:host-gateway"
              cap_drop:
                - ALL
              security_opt:
                - "label=disable"
              ulimits:
                - name: nofile
                  soft: 1024
                  hard: 2048
        "#;
        let daemon: Daemon = Config::builder()
            .add_source(File::from_str(input, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let podman = daemon.podman.expect("podman block");
        assert!(podman.socket_path.is_none(), "socket path is auto-detected by default");
        assert_eq!(podman.network_mode.as_deref(), Some("host"));
        assert_eq!(
            podman.extra_hosts,
            Some(vec!["host.containers.internal:host-gateway".to_string()])
        );
        assert_eq!(podman.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(podman.security_opt, Some(vec!["label=disable".to_string()]));
        assert_eq!(podman.ulimits.map(|ulimits| ulimits.len()), Some(1));
        assert!(daemon.docker.is_none());
    }

    #[test]
    fn deserialize_https_proxy_ca_from_indexed_map() {
        let input = r#"
            [https_proxy_ca]
            1 = "/ca/b.pem"
            0 = "/ca/a.pem"
        "#;
        let cfg: ProxyCaOnly = config::Config::builder()
            .add_source(config::File::from_str(input, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            cfg.https_proxy_ca,
            Some(vec!["/ca/a.pem".to_string(), "/ca/b.pem".to_string()])
        );
    }
}

