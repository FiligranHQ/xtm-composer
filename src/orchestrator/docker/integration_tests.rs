//! End-to-end tests against a real engine and an authenticated registry, ignored by
//! default. The integration workflow runs them on Docker, rootful Podman and rootless Podman.
//!
//! - `XTM_IT_ENGINE`: `docker` or `podman`
//! - `XTM_IT_SOCKET`: engine socket, optional (Podman auto-detects it)
//! - `XTM_IT_PODMAN_MODE`: expected Podman mode, `rootful` or `rootless`
//! - `XTM_IT_REGISTRY`, `XTM_IT_REGISTRY_USERNAME`, `XTM_IT_REGISTRY_PASSWORD`: registry with htpasswd authentication
//! - `XTM_IT_IMAGE`: image of that registry printing [`LOG_MARKER`], then running until stopped

use super::{ContainerEngine, DockerOrchestrator};
use crate::api::{ApiConnector, ApiContractConfig, ConnectorStatus};
use crate::config::settings::{Daemon, Docker as DockerOptions, Registry};
use crate::orchestrator::image::Image;
use crate::orchestrator::podman::{self, PodmanMode};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bollard::query_parameters::CreateImageOptions;
use futures::TryStreamExt;
use std::time::{Duration, Instant};

const LOG_MARKER: &str = "xtm-composer-it ready";
const WAIT_TIMEOUT: Duration = Duration::from_secs(90);

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{} must be set", name))
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn daemon(username: Option<String>, password: Option<String>) -> Daemon {
    let options = DockerOptions {
        socket_path: optional_env("XTM_IT_SOCKET"),
        ..Default::default()
    };
    Daemon {
        selector: env("XTM_IT_ENGINE"),
        registry: Some(Registry {
            server: Some(env("XTM_IT_REGISTRY")),
            username,
            password,
            email: None,
        }),
        portainer: None,
        kubernetes: None,
        docker: Some(options.clone()),
        swarm: None,
        podman: Some(options),
    }
}

fn daemon_with_credentials() -> Daemon {
    daemon(
        Some(env("XTM_IT_REGISTRY_USERNAME")),
        Some(env("XTM_IT_REGISTRY_PASSWORD")),
    )
}

async fn orchestrator(daemon: &Daemon) -> DockerOrchestrator {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    let retry_interval = Duration::from_secs(1);
    let connect = async {
        match daemon.selector.as_str() {
            "docker" => DockerOrchestrator::new(daemon, retry_interval).await,
            "podman" => podman::connect(daemon, retry_interval).await,
            other => panic!("unsupported XTM_IT_ENGINE '{}'", other),
        }
    };
    tokio::time::timeout(Duration::from_secs(30), connect)
        .await
        .expect("the engine API must answer")
}

fn connector(test: &str) -> ApiConnector {
    let name = format!(
        "xtm-it-{}-{}-{}",
        env("XTM_IT_ENGINE"),
        test,
        std::process::id()
    );
    ApiConnector {
        id: format!("{}-id", name),
        platform: "opencti".to_string(),
        name,
        image: env("XTM_IT_IMAGE"),
        contract_hash: "hash-1".to_string(),
        current_status: None,
        requested_status: "starting".to_string(),
        contract_configuration: vec![ApiContractConfig {
            key: "XTM_IT_VALUE".to_string(),
            value: "a=b".to_string(),
            is_sensitive: false,
        }],
        undecryptable_keys: vec![],
    }
}

async fn remove_leftover(orchestrator: &DockerOrchestrator, connector: &ApiConnector) {
    if let Some(container) = orchestrator.get(connector).await {
        orchestrator.remove(&container).await;
    }
}

async fn wait_for_status(
    orchestrator: &DockerOrchestrator,
    connector: &ApiConnector,
    expected: ConnectorStatus,
) -> OrchestratorContainer {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let container = orchestrator.get(connector).await;
        if let Some(container) = &container
            && orchestrator.state_converter(container) == expected
        {
            return container.clone();
        }
        assert!(
            Instant::now() < deadline,
            "connector never reached {:?}, last seen: {:?}",
            expected,
            container
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_logs(
    orchestrator: &DockerOrchestrator,
    container: &OrchestratorContainer,
    connector: &ApiConnector,
) -> Vec<String> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let logs = orchestrator
            .logs(container, connector)
            .await
            .unwrap_or_default();
        if logs.iter().any(|line| line.contains(LOG_MARKER)) {
            return logs;
        }
        assert!(
            Instant::now() < deadline,
            "log marker never seen, logs: {:?}",
            logs
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn listed<'a>(
    containers: &'a [OrchestratorContainer],
    connector: &ApiConnector,
) -> &'a OrchestratorContainer {
    containers
        .iter()
        .find(|container| container.name == connector.container_name())
        .unwrap_or_else(|| {
            panic!(
                "{} not listed in {:?}",
                connector.container_name(),
                containers
            )
        })
}

#[tokio::test]
#[ignore = "requires a container engine and an authenticated registry"]
async fn engine_and_podman_mode_are_detected() {
    let orchestrator = orchestrator(&daemon(None, None)).await;
    let compat = orchestrator.compat.as_ref().expect("compatible API client");
    match env("XTM_IT_ENGINE").as_str() {
        "docker" => assert_eq!(orchestrator.engine, ContainerEngine::Docker),
        _ => {
            assert_eq!(orchestrator.engine, ContainerEngine::Podman);
            let version = compat.version().await.expect("engine version");
            assert!(podman::podman_version(&version).is_some(), "{:?}", version);
            let mode = podman::detect_mode(&compat.info().await.expect("engine info"));
            let expected = match env("XTM_IT_PODMAN_MODE").as_str() {
                "rootless" => PodmanMode::Rootless,
                _ => PodmanMode::Rootful,
            };
            assert_eq!(mode, expected);
        }
    }
}

/// Deploy, start, stop, logs, refresh and removal of a connector whose image comes from
/// a registry requiring credentials.
#[tokio::test]
#[ignore = "requires a container engine and an authenticated registry"]
async fn connector_lifecycle_with_private_registry_credentials() {
    let orchestrator = orchestrator(&daemon_with_credentials()).await;
    let mut connector = connector("lifecycle");
    remove_leftover(&orchestrator, &connector).await;

    let deployed = orchestrator
        .deploy(&connector)
        .await
        .expect("connector deployed");
    assert!(deployed.is_managed());
    assert_eq!(deployed.extract_workload_id(), connector.id);
    assert_eq!(deployed.extract_config_hash(), "hash-1");
    assert_eq!(
        deployed.envs.get("XTM_IT_VALUE").map(String::as_str),
        Some("a=b")
    );
    for platform_label in ["xtm-platform", "opencti-platform"] {
        assert_eq!(
            deployed.labels.get(platform_label).map(String::as_str),
            Some("opencti")
        );
    }
    for hash_env in ["XTM_CONFIG_HASH", "OPENCTI_CONFIG_HASH"] {
        assert_eq!(deployed.envs.get(hash_env).map(String::as_str), Some("hash-1"));
    }

    let running = wait_for_status(&orchestrator, &connector, ConnectorStatus::Started).await;
    assert!(running.started_at.is_some());
    let containers = orchestrator.list().await;
    assert_eq!(
        containers
            .iter()
            .filter(|container| container.name == connector.container_name())
            .count(),
        1,
        "a workload carrying the current and legacy manager labels is listed once"
    );
    assert_eq!(
        listed(&containers, &connector).extract_workload_id(),
        connector.id
    );
    wait_for_logs(&orchestrator, &running, &connector).await;

    orchestrator.stop(&running, &connector).await;
    let stopped = wait_for_status(&orchestrator, &connector, ConnectorStatus::Stopped).await;
    let containers = orchestrator.list().await;
    assert_eq!(
        orchestrator.state_converter(listed(&containers, &connector)),
        ConnectorStatus::Stopped
    );
    assert!(orchestrator.logs(&stopped, &connector).await.is_some());

    orchestrator.start(&stopped, &connector).await;
    wait_for_status(&orchestrator, &connector, ConnectorStatus::Started).await;

    connector.contract_hash = "hash-2".to_string();
    let refreshed = orchestrator
        .refresh(&connector)
        .await
        .expect("connector refreshed");
    assert_ne!(refreshed.id, running.id);
    assert_eq!(refreshed.extract_config_hash(), "hash-2");
    let refreshed = wait_for_status(&orchestrator, &connector, ConnectorStatus::Started).await;

    orchestrator.remove(&refreshed).await;
    assert!(orchestrator.get(&connector).await.is_none());
    assert!(
        orchestrator
            .list()
            .await
            .iter()
            .all(|container| container.name != connector.container_name())
    );
}

#[tokio::test]
#[ignore = "requires a container engine and an authenticated registry"]
async fn deploy_fails_with_wrong_registry_credentials() {
    let daemon = daemon(
        Some(env("XTM_IT_REGISTRY_USERNAME")),
        Some("wrong-password".to_string()),
    );
    let orchestrator = orchestrator(&daemon).await;
    let connector = connector("bad-credentials");
    remove_leftover(&orchestrator, &connector).await;

    assert!(orchestrator.deploy(&connector).await.is_none());
    assert!(orchestrator.get(&connector).await.is_none());

    let resolver = Image::new(daemon.registry.clone());
    let error = orchestrator
        .pull_image(
            &resolver.build_name(connector.image.clone()),
            resolver.get_credentials(),
        )
        .await
        .expect_err("the pull must be rejected");
    assert!(error.to_lowercase().contains("auth"), "{}", error);
}

/// Why pulls with credentials do not go through bollard. Once this fails, bollard
/// encodes `X-Registry-Auth` as base64url and the compatible API pull can be dropped.
#[tokio::test]
#[ignore = "requires a container engine and an authenticated registry"]
async fn bollard_registry_auth_header_is_rejected_for_some_passwords() {
    let daemon = daemon_with_credentials();
    let orchestrator = orchestrator(&daemon).await;
    let resolver = Image::new(daemon.registry.clone());
    let credentials = resolver.get_credentials().expect("registry credentials");
    let bollard_header = STANDARD.encode(serde_json::to_string(&credentials).unwrap());
    assert!(
        bollard_header.contains('+') || bollard_header.contains('/'),
        "XTM_IT_REGISTRY_PASSWORD must produce '+' or '/' in standard base64: {}",
        bollard_header
    );

    let pull = orchestrator
        .docker
        .create_image(
            Some(CreateImageOptions {
                from_image: Some(resolver.build_name(env("XTM_IT_IMAGE"))),
                ..Default::default()
            }),
            None,
            Some(credentials),
        )
        .try_collect::<Vec<_>>()
        .await;
    assert!(pull.is_err(), "bollard now pulls with these credentials");
}

/// Only the registry server is configured: the engine authenticates with the
/// credentials stored on its host (`podman login`). Run after that login.
#[tokio::test]
#[ignore = "requires registry credentials stored on the Podman host"]
async fn delegated_registry_authentication_to_the_podman_host() {
    let orchestrator = orchestrator(&daemon(None, None)).await;
    let connector = connector("delegated-auth");
    remove_leftover(&orchestrator, &connector).await;

    orchestrator
        .deploy(&connector)
        .await
        .expect("connector deployed");
    let running = wait_for_status(&orchestrator, &connector, ConnectorStatus::Started).await;
    wait_for_logs(&orchestrator, &running, &connector).await;

    orchestrator.remove(&running).await;
    assert!(orchestrator.get(&connector).await.is_none());
}
