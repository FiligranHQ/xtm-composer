pub mod openaev;
pub mod opencti;

use crate::api::ComposerApi;
use crate::config::settings::{Daemon, OpenCTI, Registry};
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::podman;
use crate::orchestrator::portainer::docker::PortainerDockerOrchestrator;
use crate::orchestrator::swarm::SwarmOrchestrator;
use crate::orchestrator::{Orchestrator, composer};
use crate::settings;
use crate::system::signals;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time::interval;

/// Before the registry was resolved per platform, all platforms deployed on
/// Kubernetes shared a single registry secret, named after the server only. The
/// deployments created back then reference it until they are refreshed, so one
/// platform keeps maintaining it: OpenCTI when it runs on Kubernetes, the other
/// platform otherwise. Like the legacy labels (#136), it will be dropped in a
/// future version.
fn maintains_legacy_registry_secret(platform: &str, opencti: &OpenCTI) -> bool {
    platform == "opencti" || !(opencti.enable && opencti.daemon.selector == "kubernetes")
}

/// Registry the legacy secret is built from, for the platform in charge of it:
/// the OpenCTI one, which all platforms used back then.
fn legacy_registry(platform: &str, opencti: &OpenCTI) -> Option<Registry> {
    if maintains_legacy_registry_secret(platform, opencti) {
        opencti.daemon.registry.clone()
    } else {
        None
    }
}

async fn build_orchestrator(
    platform: &'static str,
    daemon_configuration: &Daemon,
) -> Box<dyn Orchestrator + Send + Sync> {
    // Delay between two attempts to reach the Docker or Podman API at startup
    let retry_interval = Duration::from_secs(settings().manager.execute_schedule.max(1));
    match daemon_configuration.selector.as_str() {
        "portainer" => match daemon_configuration.portainer.clone() {
            Some(config) => match config.env_type.as_str() {
                "docker" => Box::new(PortainerDockerOrchestrator::new(
                    config,
                    daemon_configuration.registry.clone(),
                )),
                def => panic!("Invalid portainer type configuration: {}", def),
            },
            None => panic!("Missing portainer configuration"),
        },
        "kubernetes" => match daemon_configuration.kubernetes.clone() {
            Some(config) => Box::new(
                KubeOrchestrator::new(
                    config,
                    daemon_configuration.registry.clone(),
                    platform,
                    legacy_registry(platform, &settings().opencti),
                )
                .await,
            ),
            None => panic!("Missing kubernetes configuration"),
        },
        "docker" => Box::new(DockerOrchestrator::new(daemon_configuration, retry_interval).await),
        "podman" => Box::new(podman::connect(daemon_configuration, retry_interval).await),
        "swarm" => match daemon_configuration.swarm.clone() {
            Some(config) => Box::new(SwarmOrchestrator::new(
                config,
                daemon_configuration.registry.clone(),
            )),
            None => panic!("Missing swarm configuration"),
        },
        def => panic!("Invalid daemon configuration: {}", def),
    }
}

async fn orchestration(api: Box<dyn ComposerApi + Send + Sync>) {
    let settings = settings();
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            // Get current deployment in target orchestrator
            let orchestrator = build_orchestrator(api.platform(), api.daemon()).await;
            // Created once the orchestrator is ready: ticks missed while it waits for its socket would run back-to-back
            let mut interval = interval(Duration::from_secs(settings.manager.execute_schedule));
            let mut log_ticks = HashMap::new();
            let mut health_tick = Instant::now();
            loop {
                interval.tick().await; // Wait for period
                composer::orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}

pub async fn alive(api: Box<dyn ComposerApi + Send + Sync>) -> JoinHandle<()> {
    let settings = settings();
    let mut interval = interval(Duration::from_secs(settings.manager.ping_alive_schedule));
    tokio::spawn(async move {
        // Start scheduling
        tokio::select! {
            _ = signals::handle_stop_signals() => {}
            _ = async {
                // Infinite retry loop for initial connection
                loop {
                    let version = api.version().await;
                    match version {
                        Some(version) => {
                            // Connection successful - register and start ping loop
                            api.register().await;
                            let mut detected_version: String = version.clone();
                            loop {
                                let ping_response = api.ping_alive().await;
                                match ping_response {
                                    Some(platform_version) => {
                                        // Register when version changes
                                        if platform_version != detected_version {
                                            api.register().await;
                                            detected_version = platform_version;
                                        }
                                    }
                                    _ => {
                                        // Connection lost - break to outer retry loop
                                        break;
                                    }
                                }
                                interval.tick().await;
                            }
                        },
                        None => {
                            // Connection failed - wait and retry
                            interval.tick().await;
                        }
                    }
                }
            } => {
                // This branch will never be reached due to the infinite loop.
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::Settings;
    use config::{Config, File, FileFormat};

    fn opencti(enable: bool, selector: &str) -> OpenCTI {
        let settings: Settings = Config::builder()
            .add_source(File::from_str(
                include_str!("../../config/default.yaml"),
                FileFormat::Yaml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let mut opencti = settings.opencti;
        opencti.enable = enable;
        opencti.daemon.selector = selector.to_string();
        opencti
    }

    #[test]
    fn opencti_maintains_the_legacy_registry_secret_when_on_kubernetes() {
        let opencti = opencti(true, "kubernetes");
        assert!(maintains_legacy_registry_secret("opencti", &opencti));
        assert!(!maintains_legacy_registry_secret("openaev", &opencti));
    }

    /// Without OpenCTI on Kubernetes, nothing would update the legacy secret
    /// still referenced by the OpenAEV deployments created before.
    #[test]
    fn openaev_maintains_the_legacy_registry_secret_when_opencti_is_not_on_kubernetes() {
        assert!(maintains_legacy_registry_secret(
            "openaev",
            &opencti(true, "docker")
        ));
        assert!(maintains_legacy_registry_secret(
            "openaev",
            &opencti(false, "kubernetes")
        ));
    }

    /// OpenAEV keeps the legacy secret with the OpenCTI registry, the one the
    /// deployments created before reference, whatever its own registry.
    #[test]
    fn legacy_registry_secret_is_maintained_with_the_opencti_registry() {
        let mut opencti = opencti(true, "docker");
        opencti.daemon.registry = Some(Registry {
            server: Some("cti.example.com".to_string()),
            username: Some("cti-user".to_string()),
            password: Some("cti-password".to_string()),
            email: None,
        });
        let registry = legacy_registry("openaev", &opencti).expect("legacy registry");
        assert_eq!(registry.server.as_deref(), Some("cti.example.com"));
        assert_eq!(registry.username.as_deref(), Some("cti-user"));

        opencti.daemon.selector = "kubernetes".to_string();
        assert!(legacy_registry("openaev", &opencti).is_none());
        assert!(legacy_registry("opencti", &opencti).is_some());
    }
}
