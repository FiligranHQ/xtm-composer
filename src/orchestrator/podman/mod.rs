use crate::config::settings::Daemon;
use crate::orchestrator::docker::compat::{CompatApi, EngineInfo, EngineVersion};
use crate::orchestrator::docker::docker::CLIENT_TIMEOUT;
use crate::orchestrator::docker::{ContainerEngine, DockerOrchestrator};
use bollard::{API_DEFAULT_VERSION, ClientVersion, Docker};
use std::fmt;
use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

const ROOTFUL_SOCKET: &str = "/run/podman/podman.sock";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PodmanMode {
    Rootful,
    Rootless,
}

impl fmt::Display for PodmanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PodmanMode::Rootful => write!(f, "rootful"),
            PodmanMode::Rootless => write!(f, "rootless"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PodmanSocket {
    pub path: String,
    pub origin: &'static str,
}

/// Sockets probed, in order, when no socket path is configured: the rootless socket of
/// the user running the Composer first, then the rootful one.
pub fn socket_candidates(xdg_runtime_dir: Option<&str>, uid: Option<u32>) -> Vec<PodmanSocket> {
    let mut candidates = Vec::new();
    if let Some(dir) = xdg_runtime_dir.filter(|dir| !dir.trim().is_empty()) {
        candidates.push(PodmanSocket {
            path: format!("{}/podman/podman.sock", dir.trim().trim_end_matches('/')),
            origin: "rootless socket ($XDG_RUNTIME_DIR)",
        });
    }
    if let Some(uid) = uid.filter(|uid| *uid != 0) {
        let path = format!("/run/user/{}/podman/podman.sock", uid);
        if candidates.iter().all(|candidate| candidate.path != path) {
            candidates.push(PodmanSocket {
                path,
                origin: "rootless socket (/run/user/<uid>)",
            });
        }
    }
    candidates.push(PodmanSocket {
        path: ROOTFUL_SOCKET.to_string(),
        origin: "rootful socket",
    });
    candidates
}

/// The configured socket, or the first existing candidate. Errs with the paths checked.
pub fn resolve_socket(
    configured: Option<&str>,
    candidates: Vec<PodmanSocket>,
    exists: impl Fn(&str) -> bool,
) -> Result<PodmanSocket, Vec<String>> {
    if let Some(path) = configured.map(str::trim).filter(|path| !path.is_empty()) {
        let path = path.trim_start_matches("unix://").to_string();
        return if exists(&path) {
            Ok(PodmanSocket {
                path,
                origin: "configured socket_path",
            })
        } else {
            Err(vec![path])
        };
    }
    let checked = candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    candidates
        .into_iter()
        .find(|candidate| exists(&candidate.path))
        .ok_or(checked)
}

/// Podman runs rootless when its API reports the `name=rootless` security option.
pub fn detect_mode(info: &EngineInfo) -> PodmanMode {
    let rootless = info
        .security_options
        .iter()
        .flatten()
        .any(|option| option.split(',').any(|part| part.trim() == "name=rootless"));
    if rootless {
        PodmanMode::Rootless
    } else {
        PodmanMode::Rootful
    }
}

/// Version of the Podman engine serving the API, `None` when it is another engine.
pub fn podman_version(version: &EngineVersion) -> Option<String> {
    version
        .components
        .iter()
        .flatten()
        .find(|component| {
            component
                .name
                .as_deref()
                .is_some_and(|name| name.to_lowercase().contains("podman"))
        })
        .map(|component| {
            component
                .version
                .clone()
                .or_else(|| version.version.clone())
                .unwrap_or_default()
        })
}

/// The API version advertised by the engine, capped to the one bollard implements.
pub fn negotiated_api_version(server_api_version: Option<&str>) -> ClientVersion {
    server_api_version
        .and_then(|version| {
            let (major, minor) = version.trim().split_once('.')?;
            Some(ClientVersion {
                major_version: major.parse().ok()?,
                minor_version: minor.parse().ok()?,
            })
        })
        .filter(|version| *version < *API_DEFAULT_VERSION)
        .unwrap_or(*API_DEFAULT_VERSION)
}

#[cfg(unix)]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

#[cfg(not(unix))]
fn current_uid() -> Option<u32> {
    None
}

async fn wait_for_socket(configured: Option<&str>, retry_interval: Duration) -> PodmanSocket {
    loop {
        let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
        let candidates = socket_candidates(xdg_runtime_dir.as_deref(), current_uid());
        match resolve_socket(configured, candidates, |path| Path::new(path).exists()) {
            Ok(socket) => return socket,
            Err(checked) => {
                error!(
                    checked = ?checked,
                    retry_in_seconds = retry_interval.as_secs(),
                    "No Podman API socket found. Enable it with `systemctl enable --now podman.socket` (rootful) or `systemctl --user enable --now podman.socket` and `loginctl enable-linger <user>` (rootless), mount it into the Composer container, or set daemon.podman.socket_path"
                );
                tokio::time::sleep(retry_interval).await;
            }
        }
    }
}

/// Connect to the Podman API socket of the daemon configuration, waiting for it to exist.
pub async fn connect(daemon: &Daemon, retry_interval: Duration) -> DockerOrchestrator {
    if cfg!(not(unix)) {
        panic!(
            "The podman daemon selector requires a Linux host: Podman is reached through a Unix socket"
        );
    }
    let options = daemon.podman.clone().unwrap_or_default();
    if let Some(path) = options.socket_path.as_deref()
        && path.contains("://")
        && !path.starts_with("unix://")
    {
        panic!(
            "Invalid daemon.podman.socket_path '{}': only Unix sockets are supported",
            path
        );
    }
    let socket = wait_for_socket(options.socket_path.as_deref(), retry_interval).await;

    let probe = CompatApi::new(&socket.path, None)
        .unwrap_or_else(|err| panic!("Cannot reach the Podman socket: {}", err));
    let version = match probe.version().await {
        Ok(version) => Some(version),
        Err(err) => {
            warn!(
                socket = socket.path,
                error = err,
                "Could not query the Podman version: check that the socket is a Podman API socket the Composer user can read and write"
            );
            None
        }
    };
    let engine_version = version.as_ref().and_then(podman_version);
    if let Some(version) = &version
        && engine_version.is_none()
    {
        warn!(
            socket = socket.path,
            engine = ?version.components.iter().flatten().filter_map(|c| c.name.clone()).collect::<Vec<_>>(),
            "The socket is not served by Podman: use the docker selector for a Docker daemon"
        );
    }
    let mode = match probe.info().await {
        Ok(info) => Some(detect_mode(&info)),
        Err(err) => {
            warn!(
                socket = socket.path,
                error = err,
                "Could not detect the Podman mode"
            );
            None
        }
    };

    let api_version =
        negotiated_api_version(version.as_ref().and_then(|v| v.api_version.as_deref()));
    let docker = Docker::connect_with_socket(&socket.path, CLIENT_TIMEOUT, &api_version)
        .unwrap_or_else(|err| panic!("Cannot connect to the Podman socket: {}", err));
    let compat = CompatApi::new(&socket.path, Some(api_version.to_string()))
        .unwrap_or_else(|err| panic!("Cannot reach the Podman socket: {}", err));

    info!(
        socket = socket.path,
        socket_origin = socket.origin,
        mode = mode
            .map(|mode| mode.to_string())
            .unwrap_or_else(|| "unknown".into()),
        podman_version = engine_version.unwrap_or_else(|| "unknown".into()),
        api_version = api_version.to_string(),
        "Connected to Podman"
    );
    DockerOrchestrator::with_engine(
        docker,
        ContainerEngine::Podman,
        Some(compat),
        Some(options),
        daemon.registry.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::docker::compat::EngineComponent;

    fn paths(candidates: &[PodmanSocket]) -> Vec<&str> {
        candidates
            .iter()
            .map(|candidate| candidate.path.as_str())
            .collect()
    }

    #[test]
    fn candidates_probe_rootless_sockets_before_the_rootful_one() {
        let candidates = socket_candidates(Some("/run/user/1000/"), Some(1000));
        assert_eq!(
            paths(&candidates),
            vec![
                "/run/user/1000/podman/podman.sock",
                "/run/podman/podman.sock"
            ],
            "the uid fallback is not repeated when $XDG_RUNTIME_DIR points to it"
        );

        let candidates = socket_candidates(Some("/custom/runtime"), Some(1001));
        assert_eq!(
            paths(&candidates),
            vec![
                "/custom/runtime/podman/podman.sock",
                "/run/user/1001/podman/podman.sock",
                "/run/podman/podman.sock"
            ]
        );
    }

    #[test]
    fn candidates_without_user_runtime_dir_fall_back_to_the_uid_then_rootful() {
        assert_eq!(
            paths(&socket_candidates(None, Some(1000))),
            vec![
                "/run/user/1000/podman/podman.sock",
                "/run/podman/podman.sock"
            ]
        );
        assert_eq!(
            paths(&socket_candidates(Some(" "), Some(0))),
            vec!["/run/podman/podman.sock"],
            "root in a container only probes the rootful socket"
        );
        assert_eq!(
            paths(&socket_candidates(None, None)),
            vec!["/run/podman/podman.sock"]
        );
    }

    #[test]
    fn resolve_picks_the_first_existing_candidate() {
        let socket = resolve_socket(
            None,
            socket_candidates(Some("/run/user/1000"), Some(1000)),
            |path| path == "/run/podman/podman.sock",
        )
        .unwrap();
        assert_eq!(socket.path, "/run/podman/podman.sock");
        assert_eq!(socket.origin, "rootful socket");

        let socket = resolve_socket(
            None,
            socket_candidates(Some("/run/user/1000"), Some(1000)),
            |_| true,
        )
        .unwrap();
        assert_eq!(socket.path, "/run/user/1000/podman/podman.sock");
    }

    #[test]
    fn resolve_reports_every_path_checked_when_none_exists() {
        let checked =
            resolve_socket(None, socket_candidates(None, Some(1000)), |_| false).unwrap_err();
        assert_eq!(
            checked,
            vec![
                "/run/user/1000/podman/podman.sock",
                "/run/podman/podman.sock"
            ]
        );
    }

    #[test]
    fn resolve_uses_the_configured_socket_only() {
        let socket = resolve_socket(
            Some("unix:///srv/podman.sock"),
            socket_candidates(None, Some(1000)),
            |path| path == "/srv/podman.sock",
        )
        .unwrap();
        assert_eq!(socket.path, "/srv/podman.sock");
        assert_eq!(socket.origin, "configured socket_path");

        let checked = resolve_socket(
            Some("/srv/missing.sock"),
            socket_candidates(None, Some(1000)),
            |path| path == "/run/podman/podman.sock",
        )
        .unwrap_err();
        assert_eq!(
            checked,
            vec!["/srv/missing.sock"],
            "no silent fallback to auto-detection"
        );

        let socket = resolve_socket(Some("  "), socket_candidates(None, None), |_| true).unwrap();
        assert_eq!(
            socket.path, "/run/podman/podman.sock",
            "a blank path means auto-detection"
        );
    }

    #[test]
    fn mode_is_rootless_only_when_the_engine_reports_it() {
        let info = |options: Option<Vec<&str>>| EngineInfo {
            security_options: options.map(|o| o.into_iter().map(String::from).collect()),
        };
        assert_eq!(
            detect_mode(&info(Some(vec![
                "name=seccomp,profile=default",
                "name=rootless"
            ]))),
            PodmanMode::Rootless
        );
        assert_eq!(
            detect_mode(&info(Some(vec![
                "name=selinux",
                "name=seccomp,profile=default"
            ]))),
            PodmanMode::Rootful
        );
        assert_eq!(detect_mode(&info(None)), PodmanMode::Rootful);
    }

    #[test]
    fn podman_is_recognized_from_the_version_components() {
        let version = |name: &str| EngineVersion {
            version: Some("4.9.3".into()),
            api_version: Some("1.41".into()),
            components: Some(vec![EngineComponent {
                name: Some(name.into()),
                version: Some("4.9.3".into()),
            }]),
        };
        assert_eq!(
            podman_version(&version("Podman Engine")),
            Some("4.9.3".to_string())
        );
        assert_eq!(podman_version(&version("Engine")), None);
        assert_eq!(podman_version(&EngineVersion::default()), None);
    }

    #[test]
    fn api_version_is_the_engine_one_capped_to_bollard() {
        assert_eq!(negotiated_api_version(Some("1.41")).to_string(), "1.41");
        assert_eq!(negotiated_api_version(Some("9.99")), *API_DEFAULT_VERSION);
        assert_eq!(
            negotiated_api_version(Some("garbage")),
            *API_DEFAULT_VERSION
        );
        assert_eq!(negotiated_api_version(None), *API_DEFAULT_VERSION);
    }
}
