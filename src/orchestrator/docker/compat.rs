//! Minimal client for the Docker-compatible engine API, for the calls bollard cannot make
//! reliably:
//! - it encodes `X-Registry-Auth` with the standard base64 alphabet while Docker and Podman
//!   decode it as base64url, so credentials containing some characters (`?`, `~`, `>`,
//!   non-ASCII) are rejected by Podman and silently dropped by Docker;
//! - its strict models reject the internal container states (`stopped`) that Podman
//!   releases still report through the compatible API.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;
use bollard::auth::DockerCredentials;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
// Same response timeout as bollard: once the pull has started, its stream is read until the end.
const PULL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
/// A pull stream silent for this long is stalled: the reconciliation loop must not wait for
/// it forever. Long enough for an engine committing large layers without reporting progress.
pub(crate) const PULL_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

pub struct CompatApi {
    client: reqwest::Client,
    api_version: Option<String>,
    pull_idle_timeout: Duration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct EngineVersion {
    pub version: Option<String>,
    pub api_version: Option<String>,
    pub components: Option<Vec<EngineComponent>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct EngineComponent {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct EngineInfo {
    pub security_options: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ContainerInspect {
    pub id: String,
    pub name: Option<String>,
    pub restart_count: Option<i64>,
    pub state: Option<ContainerInspectState>,
    pub config: Option<ContainerInspectConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ContainerInspectState {
    pub status: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ContainerInspectConfig {
    pub env: Option<Vec<String>>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ContainerSummary {
    pub id: String,
    pub names: Option<Vec<String>>,
    pub state: Option<String>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
struct RegistryAuth<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    serveraddress: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identitytoken: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    registrytoken: Option<&'a str>,
}

/// Encode registry credentials as the `X-Registry-Auth` header value: base64url JSON,
/// as the Docker and Podman CLIs do.
pub fn encode_registry_auth(credentials: &DockerCredentials) -> String {
    let auth = RegistryAuth {
        username: credentials.username.as_deref(),
        password: credentials.password.as_deref(),
        auth: credentials.auth.as_deref(),
        serveraddress: credentials.serveraddress.as_deref(),
        identitytoken: credentials.identitytoken.as_deref(),
        registrytoken: credentials.registrytoken.as_deref(),
    };
    URL_SAFE.encode(serde_json::to_vec(&auth).expect("registry credentials serialize to JSON"))
}

/// Map the Podman internal states to their Docker equivalent, as Podman itself does
/// in its latest releases.
pub fn normalize_state(state: &str) -> String {
    match state {
        "stopped" => "exited",
        "stopping" => "running",
        "configured" | "initialized" => "created",
        other => other,
    }
    .to_string()
}

/// Error reported in an image pull progress line, if any.
pub fn pull_stream_error(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    non_empty_str(value.pointer("/errorDetail/message"))
        .or_else(|| non_empty_str(value.get("error")))
}

/// Error message of a failed engine API response.
pub fn response_error(status: reqwest::StatusCode, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body.trim())
        .ok()
        .and_then(|value| {
            non_empty_str(value.get("message"))
                .or_else(|| non_empty_str(value.pointer("/errorDetail/message")))
                .or_else(|| non_empty_str(value.get("error")))
        })
        .unwrap_or_else(|| body.trim().to_string());
    if message.is_empty() {
        format!("engine API returned {}", status)
    } else {
        format!("engine API returned {}: {}", status, message)
    }
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

impl CompatApi {
    /// Client bound to the engine API socket. Without `api_version`, requests use the
    /// unversioned paths, which only some endpoints (`/version`, `/info`, `/images/create`) accept.
    #[cfg(unix)]
    pub fn new(socket_path: &str, api_version: Option<String>) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .unix_socket(socket_path.trim_start_matches("unix://"))
            .build()
            .map_err(|err| format!("cannot create the engine API client: {}", err))?;
        Ok(Self {
            client,
            api_version,
            pull_idle_timeout: PULL_IDLE_TIMEOUT,
        })
    }

    #[cfg(not(unix))]
    pub fn new(_socket_path: &str, _api_version: Option<String>) -> Result<Self, String> {
        Err("the engine API is reached through a Unix socket, only available on Unix hosts".into())
    }

    fn url(&self, path: &str) -> String {
        match &self.api_version {
            Some(version) => format!("http://localhost/v{}{}", version, path),
            None => format!("http://localhost{}", path),
        }
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<T>, String> {
        let response = self
            .client
            .get(self.url(path))
            .query(query)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|err| format!("engine API request failed: {}", err))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body = response
            .text()
            .await
            .map_err(|err| format!("cannot read the engine API response: {}", err))?;
        if !status.is_success() {
            return Err(response_error(status, &body));
        }
        serde_json::from_str(&body)
            .map(Some)
            .map_err(|err| format!("unexpected engine API response: {}", err))
    }

    pub async fn version(&self) -> Result<EngineVersion, String> {
        self.get_json("/version", &[])
            .await?
            .ok_or_else(|| "no /version endpoint".into())
    }

    pub async fn info(&self) -> Result<EngineInfo, String> {
        self.get_json("/info", &[])
            .await?
            .ok_or_else(|| "no /info endpoint".into())
    }

    pub async fn inspect_container(&self, name: &str) -> Result<Option<ContainerInspect>, String> {
        self.get_json(&format!("/containers/{}/json", name), &[])
            .await
    }

    pub async fn list_containers(&self, label: &str) -> Result<Vec<ContainerSummary>, String> {
        let filters = serde_json::json!({ "label": [label] }).to_string();
        Ok(self
            .get_json(
                "/containers/json",
                &[("all", "true"), ("filters", &filters)],
            )
            .await?
            .unwrap_or_default())
    }

    /// Pull an image, failing on an error reported before or during the pull stream.
    pub async fn pull_image(
        &self,
        image: &str,
        credentials: Option<&DockerCredentials>,
    ) -> Result<(), String> {
        let mut request = self
            .client
            .post(self.url("/images/create"))
            .query(&[("fromImage", image)]);
        if let Some(credentials) = credentials {
            request = request.header("X-Registry-Auth", encode_registry_auth(credentials));
        }
        let mut response = tokio::time::timeout(PULL_RESPONSE_TIMEOUT, request.send())
            .await
            .map_err(|_| "timed out waiting for the engine to start the pull".to_string())?
            .map_err(|err| format!("engine API request failed: {}", err))?;
        let status = response.status();
        if !status.is_success() {
            let body = tokio::time::timeout(self.pull_idle_timeout, response.text())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            return Err(response_error(status, &body));
        }

        let mut pending: Vec<u8> = Vec::new();
        while let Some(chunk) = tokio::time::timeout(self.pull_idle_timeout, response.chunk())
            .await
            .map_err(|_| pull_stalled_error(self.pull_idle_timeout))?
            .map_err(|err| format!("pull stream interrupted: {}", err))?
        {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                check_pull_line(image, &line)?;
            }
        }
        check_pull_line(image, &pending)
    }
}

pub(crate) fn pull_stalled_error(idle_timeout: Duration) -> String {
    format!(
        "pull stalled: no progress from the engine for {} seconds",
        idle_timeout.as_secs()
    )
}

fn check_pull_line(image: &str, line: &[u8]) -> Result<(), String> {
    let line = String::from_utf8_lossy(line);
    if line.trim().is_empty() {
        return Ok(());
    }
    if let Some(error) = pull_stream_error(&line) {
        return Err(error);
    }
    debug!(image = image, progress = line.trim(), "Pulling image");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn credentials(password: &str) -> DockerCredentials {
        DockerCredentials {
            username: Some("xtm-it".to_string()),
            password: Some(password.to_string()),
            email: Some("it@example.com".to_string()),
            serveraddress: Some("localhost:5000".to_string()),
            ..Default::default()
        }
    }

    // Password used by the integration tests: with the standard alphabet its header
    // contains '+' or '/', which base64url decoders reject.
    pub(crate) const PASSWORD_BREAKING_STANDARD_BASE64: &str = "Xtm?Composer~Test>2026!";

    #[test]
    fn standard_base64_header_breaks_base64url_decoding_for_some_passwords() {
        // What bollard sends: the standard alphabet over its full credentials JSON.
        let bollard_json =
            serde_json::to_string(&credentials(PASSWORD_BREAKING_STANDARD_BASE64)).unwrap();
        let bollard_header = STANDARD.encode(bollard_json);
        assert!(
            bollard_header.contains('+') || bollard_header.contains('/'),
            "the test password must exercise the standard/base64url mismatch: {bollard_header}"
        );
        assert!(URL_SAFE.decode(&bollard_header).is_err());
    }

    #[test]
    fn registry_auth_is_base64url_json_without_null_fields() {
        for password in [
            PASSWORD_BREAKING_STANDARD_BASE64,
            "simple",
            "p\u{e4}ss\u{fffd}~?>/+=\"\\",
        ] {
            let header = encode_registry_auth(&credentials(password));
            assert!(!header.contains('+') && !header.contains('/'), "{header}");

            let decoded: Value =
                serde_json::from_slice(&URL_SAFE.decode(&header).unwrap()).unwrap();
            assert_eq!(
                decoded,
                serde_json::json!({
                    "username": "xtm-it",
                    "password": password,
                    "serveraddress": "localhost:5000",
                })
            );
        }
    }

    #[test]
    fn podman_internal_states_are_mapped_to_docker_states() {
        assert_eq!(normalize_state("stopped"), "exited");
        assert_eq!(normalize_state("stopping"), "running");
        assert_eq!(normalize_state("configured"), "created");
        assert_eq!(normalize_state("initialized"), "created");
        for state in [
            "created",
            "running",
            "paused",
            "restarting",
            "removing",
            "exited",
            "dead",
        ] {
            assert_eq!(normalize_state(state), state);
        }
    }

    #[test]
    fn pull_stream_errors_are_detected_in_docker_and_podman_formats() {
        // Docker
        assert_eq!(
            pull_stream_error(
                r#"{"errorDetail":{"message":"unauthorized: authentication required"},"error":"unauthorized: authentication required"}"#
            ),
            Some("unauthorized: authentication required".to_string())
        );
        // Podman, once progress was reported
        assert_eq!(
            pull_stream_error(
                r#"{"progressDetail":{},"errorDetail":{"message":"reading blob: EOF"},"error":"reading blob: EOF"}"#
            ),
            Some("reading blob: EOF".to_string())
        );
        assert_eq!(
            pull_stream_error(r#"{"error":"only error"}"#),
            Some("only error".to_string())
        );
    }

    #[test]
    fn pull_stream_progress_lines_are_not_errors() {
        for line in [
            r#"{"status":"Pulling from xtm/connector","id":"1.0"}"#,
            r#"{"status":"Downloading","progressDetail":{"current":10,"total":100},"progress":"[=>   ]","id":"abc"}"#,
            r#"{"status":"Download complete","progressDetail":{},"id":"0123456789ab"}"#,
            r#"{"status":"Pulling fs layer","error":"","errorDetail":{"message":""}}"#,
            "",
            "not json",
        ] {
            assert_eq!(pull_stream_error(line), None, "{line}");
        }
    }

    #[test]
    fn response_errors_expose_the_engine_message() {
        let status = reqwest::StatusCode::BAD_REQUEST;
        // Podman
        assert_eq!(
            response_error(
                status,
                r#"{"cause":"illegal base64 data at input byte 4","message":"failed to parse \"X-Registry-Auth\" header","response":400}"#
            ),
            "engine API returned 400 Bad Request: failed to parse \"X-Registry-Auth\" header"
        );
        // Docker
        assert_eq!(
            response_error(
                reqwest::StatusCode::NOT_FOUND,
                r#"{"message":"pull access denied"}"#
            ),
            "engine API returned 404 Not Found: pull access denied"
        );
        assert_eq!(
            response_error(status, "plain text\n"),
            "engine API returned 400 Bad Request: plain text"
        );
        assert_eq!(
            response_error(status, ""),
            "engine API returned 400 Bad Request"
        );
    }

    #[test]
    fn lenient_models_accept_podman_responses() {
        let inspect: ContainerInspect = serde_json::from_str(
            r#"{"Id":"abc","Name":"/connector","RestartCount":2,
                "State":{"Status":"stopped","StartedAt":"2026-10-21T10:00:00.123456789Z","Health":{"Status":""}},
                "Config":{"Env":["A=1"],"Labels":null}}"#,
        )
        .unwrap();
        assert_eq!(inspect.state.unwrap().status.as_deref(), Some("stopped"));
        assert!(inspect.config.unwrap().labels.is_none());

        let summaries: Vec<ContainerSummary> = serde_json::from_str(
            r#"[{"Id":"abc","Names":["/connector"],"State":"stopped","Labels":{"opencti-manager":"m"}},
                {"Id":"def","Names":null,"State":"running","Labels":null}]"#,
        )
        .unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].state.as_deref(), Some("stopped"));

        let version: EngineVersion = serde_json::from_str(
            r#"{"Platform":{"Name":"linux/amd64/ubuntu-24.04"},"Components":[{"Name":"Podman Engine","Version":"4.9.3","Details":{"APIVersion":"4.9.3"}}],"Version":"4.9.3","ApiVersion":"1.41","MinAPIVersion":"1.24"}"#,
        )
        .unwrap();
        assert_eq!(version.api_version.as_deref(), Some("1.41"));

        let info: EngineInfo = serde_json::from_str(r#"{"SecurityOptions":null}"#).unwrap();
        assert!(info.security_options.is_none());
    }
}

#[cfg(all(test, unix))]
mod fake_engine_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;

    const STREAM_HEAD: &str =
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";

    /// Engine socket answering one request with `response`, then keeping the connection
    /// open for `hold`.
    struct FakeEngine {
        socket: PathBuf,
        request: Option<JoinHandle<String>>,
    }

    impl FakeEngine {
        /// Lowercased head of the request received.
        async fn request_head(&mut self) -> String {
            self.request.take().unwrap().await.unwrap()
        }
    }

    impl Drop for FakeEngine {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    fn fake_engine(name: &str, response: String, hold: Duration) -> FakeEngine {
        let socket =
            std::env::temp_dir().join(format!("xtm-composer-{}-{}.sock", name, std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let request = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buffer = [0u8; 4096];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                head.extend_from_slice(&buffer[..read]);
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(hold).await;
            String::from_utf8_lossy(&head).to_lowercase()
        });
        FakeEngine {
            socket,
            request: Some(request),
        }
    }

    fn client(engine: &FakeEngine, pull_idle_timeout: Duration) -> CompatApi {
        let mut api =
            CompatApi::new(engine.socket.to_str().unwrap(), Some("1.41".to_string())).unwrap();
        api.pull_idle_timeout = pull_idle_timeout;
        api
    }

    fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        )
    }

    fn credentials() -> DockerCredentials {
        DockerCredentials {
            username: Some("xtm-it".to_string()),
            password: Some("Xtm?Composer~Test>2026!".to_string()),
            serveraddress: Some("localhost:5000".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn pull_reads_the_whole_stream_and_sends_base64url_credentials() {
        let mut engine = fake_engine(
            "pull-ok",
            format!(
                "{}{}\n{}",
                STREAM_HEAD,
                r#"{"status":"Pulling fs layer","progressDetail":{},"id":"0123456789ab"}"#,
                r#"{"status":"Download complete","progressDetail":{},"id":"0123456789ab"}"#
            ),
            Duration::ZERO,
        );

        client(&engine, PULL_IDLE_TIMEOUT)
            .pull_image("localhost:5000/xtm/connector:1.0", Some(&credentials()))
            .await
            .unwrap();

        let head = engine.request_head().await;
        assert!(
            head.starts_with(
                "post /v1.41/images/create?fromimage=localhost%3a5000%2fxtm%2fconnector%3a1.0 "
            ),
            "{head}"
        );
        let header = encode_registry_auth(&credentials()).to_lowercase();
        assert!(
            head.contains(&format!("x-registry-auth: {header}\r\n")),
            "{head}"
        );
    }

    #[tokio::test]
    async fn pull_without_credentials_sends_no_auth_header() {
        let mut engine = fake_engine(
            "pull-anonymous",
            format!("{}{}\n", STREAM_HEAD, r#"{"status":"Download complete"}"#),
            Duration::ZERO,
        );

        client(&engine, PULL_IDLE_TIMEOUT)
            .pull_image("xtm/connector:1.0", None)
            .await
            .unwrap();

        assert!(!engine.request_head().await.contains("x-registry-auth"));
    }

    #[tokio::test]
    async fn pull_fails_on_an_error_reported_in_the_stream() {
        let engine = fake_engine(
            "pull-stream-error",
            format!(
                "{}{}\n{}\n",
                STREAM_HEAD,
                r#"{"status":"Pulling fs layer","id":"0123456789ab"}"#,
                r#"{"errorDetail":{"message":"unauthorized: authentication required"},"error":"unauthorized: authentication required"}"#
            ),
            Duration::ZERO,
        );

        let error = client(&engine, PULL_IDLE_TIMEOUT)
            .pull_image("xtm/connector:1.0", Some(&credentials()))
            .await
            .unwrap_err();

        assert_eq!(error, "unauthorized: authentication required");
    }

    #[tokio::test]
    async fn pull_fails_on_an_error_status() {
        let engine = fake_engine(
            "pull-status-error",
            json_response(
                "400 Bad Request",
                r#"{"cause":"illegal base64 data at input byte 4","message":"failed to parse \"X-Registry-Auth\" header","response":400}"#,
            ),
            Duration::ZERO,
        );

        let error = client(&engine, PULL_IDLE_TIMEOUT)
            .pull_image("xtm/connector:1.0", Some(&credentials()))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            "engine API returned 400 Bad Request: failed to parse \"X-Registry-Auth\" header"
        );
    }

    #[tokio::test]
    async fn pull_fails_on_an_error_status_whose_body_stalls() {
        let idle_timeout = Duration::from_millis(500);
        let engine = fake_engine(
            "pull-status-stall",
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"mess".to_string(),
            Duration::from_secs(10),
        );
        let started = Instant::now();

        let error = client(&engine, idle_timeout)
            .pull_image("xtm/connector:1.0", Some(&credentials()))
            .await
            .unwrap_err();

        assert_eq!(error, "engine API returned 401 Unauthorized");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn pull_fails_when_the_stream_stalls() {
        let idle_timeout = Duration::from_millis(500);
        let engine = fake_engine(
            "pull-stall",
            format!(
                "{}{}\n",
                STREAM_HEAD,
                r#"{"status":"Downloading","progressDetail":{"current":1,"total":100}}"#
            ),
            Duration::from_secs(10),
        );
        let started = Instant::now();

        let error = client(&engine, idle_timeout)
            .pull_image("xtm/connector:1.0", None)
            .await
            .unwrap_err();

        assert_eq!(error, pull_stalled_error(idle_timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn inspect_of_a_missing_container_is_none() {
        let mut engine = fake_engine(
            "inspect-missing",
            json_response("404 Not Found", r#"{"message":"no such container"}"#),
            Duration::ZERO,
        );

        let inspect = client(&engine, PULL_IDLE_TIMEOUT)
            .inspect_container("connector-a")
            .await
            .unwrap();

        assert!(inspect.is_none());
        assert!(
            engine
                .request_head()
                .await
                .starts_with("get /v1.41/containers/connector-a/json ")
        );
    }

    #[tokio::test]
    async fn list_filters_on_the_manager_label() {
        let mut engine = fake_engine(
            "list",
            json_response(
                "200 OK",
                r#"[{"Id":"abc","Names":["/connector-a"],"State":"stopped","Labels":{"opencti-manager":"m"}}]"#,
            ),
            Duration::ZERO,
        );

        let containers = client(&engine, PULL_IDLE_TIMEOUT)
            .list_containers("opencti-manager=m")
            .await
            .unwrap();

        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].state.as_deref(), Some("stopped"));
        let head = engine.request_head().await;
        assert!(
            head.starts_with(
                "get /v1.41/containers/json?all=true&filters=%7b%22label%22%3a%5b%22opencti-manager%3dm%22%5d%7d "
            ),
            "{head}"
        );
    }
}
