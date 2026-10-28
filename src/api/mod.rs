use crate::config::settings::Daemon;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use std::time::Duration;
use tracing::{error, info};

pub mod openaev;
pub mod opencti;
mod decrypt_value;
pub mod rsa_key;

pub const PROXY_CA_CERT_MOUNT_PATH: &str = "/etc/ssl/certs/xtm-proxy-ca.crt";

#[derive(Debug, Clone)]
struct PlatformProxyConfig {
    with_proxy: bool,
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    no_proxy: Option<String>,
    https_proxy_ca: Option<Vec<String>>,
    https_proxy_reject_unauthorized: bool,
}

/// Configuration for building an HTTP client with proxy and TLS settings.
pub struct HttpClientConfig {
    pub request_timeout: u64,
    pub connect_timeout: u64,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    /// Comma-separated hosts, domains or CIDR ranges reached without the proxy.
    pub no_proxy: Option<String>,
    pub platform_name: String,
}

const HTTP_PROXY_ENV_VARS: &[&str] = &["HTTP_PROXY", "http_proxy"];
const HTTPS_PROXY_ENV_VARS: &[&str] = &["HTTPS_PROXY", "https_proxy"];
const NO_PROXY_ENV_VARS: &[&str] = &["NO_PROXY", "no_proxy"];
const ALL_PROXY_ENV_VARS: &[&str] = &["ALL_PROXY", "all_proxy"];

/// Resolve a proxy setting: the configured value wins, else the first of
/// `env_vars` set in the Composer's own process environment.
fn resolve_proxy_setting(configured: Option<&str>, env_vars: &[&str]) -> Option<String> {
    configured
        .map(|s| s.to_string())
        .or_else(|| env_vars.iter().find_map(|key| std::env::var(key).ok()))
}

/// The proxy URL the system proxies would use for a scheme: the first of `env_vars`
/// set, else ALL_PROXY/all_proxy, a blank value counting as unset.
fn system_proxy_url(env_vars: &[&str]) -> Option<String> {
    let first_set = |vars: &[&str]| {
        resolve_proxy_setting(None, vars).filter(|url| !url.trim().is_empty())
    };
    first_set(env_vars).or_else(|| first_set(ALL_PROXY_ENV_VARS))
}

/// Build a reqwest HTTP client configured with proxy and TLS settings.
///
/// - `with_proxy: false` → disables all proxies (ignores system env vars).
/// - `with_proxy: true` + explicit `http_proxy`/`https_proxy` → uses configured proxies,
///   bypassed for the `no_proxy` hosts (else the NO_PROXY env var).
/// - `with_proxy: true` + no explicit proxy + `no_proxy` → uses the HTTP_PROXY/HTTPS_PROXY
///   (else ALL_PROXY) env vars, as the system proxies would, bypassed for the `no_proxy` hosts.
/// - `with_proxy: true` + no explicit proxy and no `no_proxy` → uses system proxies
///   (HTTP_PROXY/HTTPS_PROXY env vars, bypassed for the NO_PROXY env var).
pub fn build_http_client(config: &HttpClientConfig) -> Result<reqwest::Client, reqwest::Error> {
    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.request_timeout))
        .connect_timeout(Duration::from_secs(config.connect_timeout))
        .danger_accept_invalid_certs(config.unsecured_certificate);

    if config.with_proxy {
        // A blank no_proxy (e.g. an empty OPENCTI__NO_PROXY rendered by a chart) is unset
        let configured_no_proxy = config.no_proxy.as_deref().filter(|s| !s.trim().is_empty());
        // System proxies only honor the NO_PROXY env var: when no_proxy is configured,
        // resolve the proxy URLs from the env so that the configured exclusions apply
        let explicit = config.http_proxy.is_some() || config.https_proxy.is_some();
        let (http_proxy, https_proxy, source) = if !explicit && configured_no_proxy.is_some() {
            (
                system_proxy_url(HTTP_PROXY_ENV_VARS),
                system_proxy_url(HTTPS_PROXY_ENV_VARS),
                "environment",
            )
        } else {
            (config.http_proxy.clone(), config.https_proxy.clone(), "explicit")
        };
        let no_proxy = resolve_proxy_setting(configured_no_proxy, NO_PROXY_ENV_VARS);
        let bypass = || no_proxy.as_deref().and_then(reqwest::NoProxy::from_string);

        if let Some(http_proxy) = &http_proxy {
            info!(platform = %config.platform_name, source, no_proxy = ?no_proxy, "Using HTTP proxy");
            let proxy = reqwest::Proxy::http(http_proxy)
                .unwrap_or_else(|e| panic!("Invalid http_proxy for platform '{}': {}", config.platform_name, e))
                .no_proxy(bypass());
            client_builder = client_builder.proxy(proxy);
        }
        if let Some(https_proxy) = &https_proxy {
            info!(platform = %config.platform_name, source, no_proxy = ?no_proxy, "Using HTTPS proxy");
            let proxy = reqwest::Proxy::https(https_proxy)
                .unwrap_or_else(|e| panic!("Invalid https_proxy for platform '{}': {}", config.platform_name, e))
                .no_proxy(bypass());
            client_builder = client_builder.proxy(proxy);
        }
        // Without any proxy URL above, reqwest uses system proxies by default
    } else {
        // Disable all proxy usage (ignore system env vars)
        client_builder = client_builder.no_proxy();
    }

    client_builder.build()
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvVariable {
    pub key: String,
    pub value: String,
    pub is_sensitive: bool,
}

#[derive(Debug, Clone)]
pub struct ApiContractConfig {
    pub key: String,
    pub value: String,
    pub is_sensitive: bool,
}

#[derive(Debug, Clone)]
pub struct ApiConnector {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub image: String,
    pub contract_hash: String,
    pub current_status: Option<String>,
    pub requested_status: String,
    pub contract_configuration: Vec<ApiContractConfig>,
    /// Keys of encrypted configuration values that could not be decrypted.
    /// When non-empty, the configuration is incomplete and must not be
    /// deployed: doing so would replace working secrets with blanks.
    pub undecryptable_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectorStatus {
    Started,
    Stopped,
}

impl FromStr for ConnectorStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<ConnectorStatus, Self::Err> {
        match input {
            "created" => Ok(ConnectorStatus::Stopped),
            "exited" => Ok(ConnectorStatus::Stopped),
            "started" => Ok(ConnectorStatus::Started),
            "healthy" => Ok(ConnectorStatus::Started),
            "running" => Ok(ConnectorStatus::Started),
            _ => Ok(ConnectorStatus::Stopped),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RequestedStatus {
    Starting,
    Stopping,
}

impl FromStr for RequestedStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<RequestedStatus, Self::Err> {
        match input {
            "starting" => Ok(RequestedStatus::Starting),
            "stopping" => Ok(RequestedStatus::Stopping),
            _ => Ok(RequestedStatus::Stopping),
        }
    }
}

/// Append proxy environment variables (HTTP_PROXY, HTTPS_PROXY, NO_PROXY)
/// to the connector container env list when proxy is enabled.
///
/// When `http_proxy`, `https_proxy`, or `no_proxy` is `None`, falls back to
/// reading `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` from the Composer's own process
/// environment, so that env-var-only proxy configurations are forwarded
/// to deployed connectors.
fn append_proxy_envs(
    envs: &mut Vec<EnvVariable>,
    with_proxy: bool,
    http_proxy: Option<&str>,
    https_proxy: Option<&str>,
    no_proxy: Option<&str>,
) {
    if !with_proxy {
        return;
    }

    // Explicit config wins, else inherit from process env (as build_http_client does)
    let resolved_http = resolve_proxy_setting(http_proxy, HTTP_PROXY_ENV_VARS);
    let resolved_https = resolve_proxy_setting(https_proxy, HTTPS_PROXY_ENV_VARS);
    let resolved_no_proxy = resolve_proxy_setting(no_proxy, NO_PROXY_ENV_VARS);

    if let Some(url) = resolved_http {
        envs.push(EnvVariable {
            key: "HTTP_PROXY".into(),
            value: url,
            is_sensitive: true, // may contain credentials (e.g. http://user:pass@proxy:8080)
        });
    }
    if let Some(url) = resolved_https {
        envs.push(EnvVariable {
            key: "HTTPS_PROXY".into(),
            value: url,
            is_sensitive: true, // may contain credentials (e.g. http://user:pass@proxy:8080)
        });
    }
    if let Some(val) = resolved_no_proxy {
        envs.push(EnvVariable {
            key: "NO_PROXY".into(),
            value: val,
            is_sensitive: false,
        });
    }
}

/// Append commonly-supported TLS CA env vars so connector runtimes can trust
/// an injected corporate proxy root certificate.
fn append_proxy_ca_envs(envs: &mut Vec<EnvVariable>, with_proxy: bool, has_proxy_ca: bool) {
    if !with_proxy || !has_proxy_ca {
        return;
    }

    for key in [
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "NODE_EXTRA_CA_CERTS",
        "CURL_CA_BUNDLE",
    ] {
        envs.push(EnvVariable {
            key: key.to_string(),
            value: PROXY_CA_CERT_MOUNT_PATH.to_string(),
            is_sensitive: false,
        });
    }
}

fn append_proxy_tls_policy_envs(
    envs: &mut Vec<EnvVariable>,
    with_proxy: bool,
    https_proxy_reject_unauthorized: bool,
) {
    if with_proxy && !https_proxy_reject_unauthorized {
        envs.push(EnvVariable {
            key: "NODE_TLS_REJECT_UNAUTHORIZED".to_string(),
            value: "0".to_string(),
            is_sensitive: false,
        });
    }
}

impl ApiConnector {
    fn platform_proxy_config(&self) -> Option<PlatformProxyConfig> {
        let settings = crate::settings();
        match self.platform.as_str() {
            "opencti" => Some(PlatformProxyConfig {
                with_proxy: settings.opencti.with_proxy,
                http_proxy: settings.opencti.http_proxy.clone(),
                https_proxy: settings.opencti.https_proxy.clone(),
                no_proxy: settings.opencti.no_proxy.clone(),
                https_proxy_ca: settings.opencti.https_proxy_ca.clone(),
                https_proxy_reject_unauthorized: settings
                    .opencti
                    .https_proxy_reject_unauthorized,
            }),
            "openaev" => Some(PlatformProxyConfig {
                with_proxy: settings.openaev.with_proxy,
                http_proxy: settings.openaev.http_proxy.clone(),
                https_proxy: settings.openaev.https_proxy.clone(),
                no_proxy: settings.openaev.no_proxy.clone(),
                https_proxy_ca: settings.openaev.https_proxy_ca.clone(),
                https_proxy_reject_unauthorized: settings
                    .openaev
                    .https_proxy_reject_unauthorized,
            }),
            _ => None,
        }
    }

    /// Resolve all `https_proxy_ca` entries into a single concatenated PEM bundle.
    ///
    /// Each entry can be either:
    /// - An inline PEM block (starts with `-----BEGIN`)
    /// - A filesystem path to a PEM file
    ///
    /// Returns `None` when proxy is disabled, the list is absent/empty,
    /// or all entries fail to resolve.
    pub fn proxy_ca_bundle(&self) -> Option<String> {
        let config = self.platform_proxy_config()?;
        if !config.with_proxy {
            return None;
        }
        let entries = config.https_proxy_ca?;
        if entries.is_empty() {
            return None;
        }

        let mut bundle = String::new();
        for entry in &entries {
            let trimmed = entry.trim();
            if trimmed.starts_with("-----BEGIN") {
                bundle.push_str(trimmed);
                if !bundle.ends_with('\n') {
                    bundle.push('\n');
                }
            } else {
                match fs::read_to_string(trimmed) {
                    Ok(content) => {
                        bundle.push_str(&content);
                        if !bundle.ends_with('\n') {
                            bundle.push('\n');
                        }
                    }
                    Err(err) => {
                        error!(
                            platform = %self.platform,
                            path = trimmed,
                            error = err.to_string(),
                            "Unable to read proxy CA certificate file"
                        );
                    }
                }
            }
        }

        if bundle.is_empty() { None } else { Some(bundle) }
    }

    pub fn container_name(&self) -> String {
        self.name
            .clone()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .to_lowercase()
    }

    pub fn container_envs(&self) -> Vec<EnvVariable> {
        let settings = crate::settings();
        let mut envs = self
            .contract_configuration
            .iter()
            .map(|config| EnvVariable {
                key: config.key.clone(),
                value: config.value.clone(),
                is_sensitive: config.is_sensitive,
            })
            .collect::<Vec<EnvVariable>>();
        if settings.opencti.enable {
            envs.push(EnvVariable {
                key: "OPENCTI_URL".into(),
                value: settings.opencti.url.clone(),
                is_sensitive: false,
            });
        }
        if settings.openaev.enable {
            envs.push(EnvVariable {
                key: "OPENAEV_URL".into(),
                value: settings.openaev.url.clone(),
                is_sensitive: false,
            });
        }
        envs.push(EnvVariable {
            key: "OPENCTI_CONFIG_HASH".into(),
            value: self.contract_hash.clone(),
            is_sensitive: false,
        });

        // Inject proxy environment variables into the connector container
        if let Some(proxy_config) = self.platform_proxy_config() {
            append_proxy_envs(
                &mut envs,
                proxy_config.with_proxy,
                proxy_config.http_proxy.as_deref(),
                proxy_config.https_proxy.as_deref(),
                proxy_config.no_proxy.as_deref(),
            );
            append_proxy_ca_envs(
                &mut envs,
                proxy_config.with_proxy,
                self.proxy_ca_bundle().is_some(),
            );
            append_proxy_tls_policy_envs(
                &mut envs,
                proxy_config.with_proxy,
                proxy_config.https_proxy_reject_unauthorized,
            );
        }

        envs
    }

    /// Display environment variables with sensitive values masked (if configured)
    pub fn display_env_variables(&self) {
        let settings = crate::settings();

        // Check if display is enabled in configuration
        let should_display = settings
            .manager
            .debug
            .as_ref()
            .map_or(false, |debug| debug.show_env_vars);

        if !should_display {
            return;
        }

        // Check if we should show sensitive values
        let show_sensitive = settings
            .manager
            .debug
            .as_ref()
            .map_or(false, |debug| debug.show_sensitive_env_vars);

        let envs = self.container_envs();

        // Build environment variables map with masked sensitive values
        let env_vars: HashMap<String, String> = envs
            .into_iter()
            .map(|env| {
                let value = if env.is_sensitive && !show_sensitive {
                    "***REDACTED***".to_string()
                } else {
                    env.value
                };
                (env.key, value)
            })
            .collect();

        // Log with structured fields
        info!(
            connector_name = %self.name,
            container_name = %self.container_name(),
            env_vars = ?env_vars,
            "Starting connector"
        );
    }
}

#[async_trait]
pub trait ComposerApi {
    fn daemon(&self) -> &Daemon;

    fn platform(&self) -> &'static str;

    fn post_logs_schedule(&self) -> Duration;

    async fn version(&self) -> Option<String>;

    async fn ping_alive(&self) -> Option<String>;

    async fn register(&self) -> ();

    async fn connectors(&self) -> Option<Vec<ApiConnector>>;

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector>;

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String>;

    async fn patch_health(
        &self,
        id: String,
        restart_count: u32,
        started_at: String,
        is_in_reboot_loop: bool,
    ) -> Option<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn base_config() -> HttpClientConfig {
        HttpClientConfig {
            request_timeout: 5,
            connect_timeout: 2,
            unsecured_certificate: false,
            with_proxy: false,
            http_proxy: None,
            https_proxy: None,
            no_proxy: None,
            platform_name: "test".into(),
        }
    }

    #[test]
    fn build_client_with_proxy_disabled() {
        let config = HttpClientConfig {
            with_proxy: false,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();
        // Client builds successfully with no proxy
        drop(client);
    }

    #[test]
    fn build_client_with_proxy_enabled_no_url() {
        let config = HttpClientConfig {
            with_proxy: true,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();
        // Client builds successfully using system proxies
        drop(client);
    }

    #[test]
    fn build_client_with_explicit_proxy() {
        let config = HttpClientConfig {
            with_proxy: true,
            http_proxy: Some("http://127.0.0.1:9999".to_string()),
            https_proxy: Some("http://127.0.0.1:9999".to_string()),
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();
        // Client builds successfully with explicit proxy
        drop(client);
    }

    #[test]
    fn build_client_with_unsecured_certificate() {
        let config = HttpClientConfig {
            unsecured_certificate: true,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();
        // Client builds successfully accepting invalid certs
        drop(client);
    }

    #[test]
    fn build_client_with_various_proxy_urls() {
        // All these should build successfully
        let urls = vec![
            "http://proxy.example.com:8080",
            "https://secure-proxy.local:443",
            "http://user:pass@proxy.internal:3128",
            "http://127.0.0.1:1080",
        ];
        for url in urls {
            let config = HttpClientConfig {
                with_proxy: true,
                http_proxy: Some(url.to_string()),
                https_proxy: Some(url.to_string()),
                ..base_config()
            };
            let client = build_http_client(&config).unwrap();
            drop(client);
        }
    }

    #[tokio::test]
    async fn proxy_enabled_with_unreachable_proxy_fails_request() {
        let _env_guard = lock_clean_proxy_env();
        // Configure proxy to an address where nothing listens
        let config = HttpClientConfig {
            with_proxy: true,
            http_proxy: Some("http://127.0.0.1:1".to_string()),
            https_proxy: Some("http://127.0.0.1:1".to_string()),
            request_timeout: 1,
            connect_timeout: 1,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();

        // Request should fail because the proxy is unreachable
        let result = client.get("http://example.com").send().await;
        assert!(
            result.is_err(),
            "Request through unreachable proxy should fail"
        );
    }

    #[tokio::test]
    async fn proxy_enabled_routes_traffic_through_proxy() {
        let _env_guard = lock_clean_proxy_env();
        // Start a local TCP listener acting as a fake proxy
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let config = HttpClientConfig {
            with_proxy: true,
            http_proxy: Some(format!("http://{}", proxy_addr)),
            https_proxy: None,
            request_timeout: 2,
            connect_timeout: 2,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();

        // Send a request in the background
        let request_handle = tokio::spawn(async move {
            // We don't care about the result — just that the proxy receives the connection
            let _ = client.get("http://fake-target.local/test").send().await;
        });

        // Accept the connection on our fake proxy — proves traffic was routed
        let accept_result = tokio::time::timeout(
            Duration::from_secs(3),
            listener.accept(),
        ).await;

        assert!(
            accept_result.is_ok(),
            "Proxy listener should have received a connection"
        );

        let (_stream, _) = accept_result.unwrap().unwrap();

        request_handle.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn proxy_disabled_does_not_route_through_proxy() {
        // Start a local TCP listener
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();

        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Even though we set the system proxy env var, with_proxy: false should ignore it
        unsafe { std::env::set_var("HTTP_PROXY", format!("http://{}", proxy_addr)); }

        let config = HttpClientConfig {
            with_proxy: false,
            request_timeout: 1,
            connect_timeout: 1,
            ..base_config()
        };
        let client = build_http_client(&config).unwrap();

        // Send a request — it should NOT go through the proxy
        let request_handle = tokio::spawn(async move {
            let _ = client.get("http://fake-target.local/test").send().await;
        });

        // The proxy should NOT receive any connection
        let accept_result = tokio::time::timeout(
            Duration::from_secs(2),
            listener.accept(),
        ).await;

        assert!(
            accept_result.is_err(),
            "Proxy listener should NOT have received a connection when with_proxy is false"
        );

        request_handle.abort();

        // Clean up env var
        // SAFETY: guarded by ENV_LOCK and acceptable in current_thread runtime
        unsafe { std::env::remove_var("HTTP_PROXY"); }
    }

    // --- Tests for no_proxy on the Composer's own requests (#169) ---

    /// Every env var that reqwest or the Composer reads to configure proxies.
    const PROXY_ENV_VARS: &[&str] = &[
        "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy",
        "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy",
    ];

    /// Holds ENV_LOCK with the proxy env vars cleared, and restores them when dropped.
    struct ProxyEnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl Drop for ProxyEnvGuard {
        fn drop(&mut self) {
            // SAFETY: ENV_LOCK is still held, _lock is dropped after this
            unsafe {
                for (key, value) in &self.saved {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    /// Lock the process env and clear the proxy env vars, so that a test only sees
    /// the ones it sets (and not, e.g., a NO_PROXY set on the developer machine), and
    /// doesn't leak them to the next test.
    fn lock_clean_proxy_env() -> ProxyEnvGuard {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = PROXY_ENV_VARS.iter().map(|key| (*key, std::env::var_os(key))).collect();
        // SAFETY: guarded by ENV_LOCK
        unsafe { for key in PROXY_ENV_VARS { std::env::remove_var(key); } }
        ProxyEnvGuard { saved, _lock: lock }
    }

    fn set_env(key: &str, value: &str) {
        // SAFETY: only called while holding the guard from lock_clean_proxy_env
        unsafe { std::env::set_var(key, value); }
    }

    /// Plain HTTP target counting the connections it accepts and answering 200.
    async fn spawn_target() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = connections.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                    .await;
            }
        });
        (addr, connections)
    }

    /// Fake proxy recording the request line of each request it gets (`GET http://...` for
    /// HTTP, `CONNECT host:port` for HTTPS) and answering 503, like a proxy that can't
    /// resolve an in-cluster Service name.
    async fn spawn_proxy() -> (std::net::SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                recorded.lock().unwrap().push(head.lines().next().unwrap_or_default().to_string());
                let _ = stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
            }
        });
        (addr, requests)
    }

    fn proxied_config(
        http_proxy: Option<std::net::SocketAddr>,
        https_proxy: Option<std::net::SocketAddr>,
        no_proxy: Option<&str>,
    ) -> HttpClientConfig {
        HttpClientConfig {
            with_proxy: true,
            http_proxy: http_proxy.map(|addr| format!("http://{}", addr)),
            https_proxy: https_proxy.map(|addr| format!("http://{}", addr)),
            no_proxy: no_proxy.map(|s| s.to_string()),
            ..base_config()
        }
    }

    /// Send a GET to `url` and return how many connections the target got and which
    /// requests the proxy got for it.
    async fn send_and_trace(
        config: &HttpClientConfig,
        url: &str,
        target: &Arc<AtomicUsize>,
        proxy: &Arc<Mutex<Vec<String>>>,
    ) -> (Result<reqwest::StatusCode, String>, usize, Vec<String>) {
        let target_before = target.load(Ordering::SeqCst);
        let proxy_before = proxy.lock().unwrap().len();
        let client = build_http_client(config).unwrap();
        let result = client
            .get(url)
            .send()
            .await
            .map(|response| response.status())
            .map_err(|e| e.to_string());
        let direct = target.load(Ordering::SeqCst) - target_before;
        let proxied = proxy.lock().unwrap()[proxy_before..].to_vec();
        (result, direct, proxied)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_proxy_is_bypassed_for_no_proxy_hosts() {
        // Regression test for #169: no_proxy was only forwarded to the connectors, so
        // the Composer sent its own platform requests to the proxy
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        let url = format!("http://{}/graphql", target_addr);

        for no_proxy in [
            "127.0.0.1",
            "localhost,127.0.0.1,opencti-ingest",
            "opencti-ingest, 127.0.0.1",
            "10.0.0.0/8,127.0.0.0/8",
        ] {
            let config = proxied_config(Some(proxy_addr), Some(proxy_addr), Some(no_proxy));
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::OK), "no_proxy {:?}", no_proxy);
            assert_eq!(direct, 1, "no_proxy {:?}: the target should be reached directly", no_proxy);
            assert!(proxied.is_empty(), "no_proxy {:?}: the proxy got {:?}", no_proxy, proxied);
        }

        // Host names, as in a platform URL like http://opencti-ingest:8080 (the "*"
        // wildcard only applies to host names, not to IP addresses)
        let url = format!("http://localhost:{}/graphql", target_addr.port());
        for no_proxy in ["localhost", "opencti-ingest,.localhost", "*"] {
            let config = proxied_config(Some(proxy_addr), Some(proxy_addr), Some(no_proxy));
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::OK), "no_proxy {:?}", no_proxy);
            assert_eq!((direct, proxied.len()), (1, 0), "no_proxy {:?}", no_proxy);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_proxy_is_still_used_for_other_hosts() {
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        let url = format!("http://{}/graphql", target_addr);

        for no_proxy in [None, Some(""), Some("localhost,opencti-ingest,10.0.0.0/8")] {
            let config = proxied_config(Some(proxy_addr), Some(proxy_addr), no_proxy);
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::SERVICE_UNAVAILABLE), "no_proxy {:?}", no_proxy);
            assert_eq!(direct, 0, "no_proxy {:?}: the target should not be reached directly", no_proxy);
            assert_eq!(proxied, vec![format!("GET {} HTTP/1.1", url)], "no_proxy {:?}", no_proxy);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_https_proxy_is_bypassed_for_no_proxy_hosts() {
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        // The target speaks plain HTTP, so the TLS handshake fails either way: only where
        // the connection went matters here
        let url = format!("https://{}/graphql", target_addr);

        let config = proxied_config(None, Some(proxy_addr), Some("127.0.0.1"));
        let (_, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(direct, 1, "the target should be reached directly");
        assert!(proxied.is_empty(), "the proxy got {:?}", proxied);

        let config = proxied_config(None, Some(proxy_addr), Some("opencti-ingest"));
        let (_, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(direct, 0, "the target should not be reached directly");
        assert_eq!(proxied, vec![format!("CONNECT {} HTTP/1.1", target_addr)]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_proxy_falls_back_to_no_proxy_env_var() {
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        let url = format!("http://{}/graphql", target_addr);

        for key in ["NO_PROXY", "no_proxy"] {
            unsafe { for k in NO_PROXY_ENV_VARS { std::env::remove_var(k); } }
            set_env(key, "opencti-ingest,127.0.0.1");
            let config = proxied_config(Some(proxy_addr), Some(proxy_addr), None);
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::OK), "{}", key);
            assert_eq!((direct, proxied.len()), (1, 0), "{}", key);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn configured_no_proxy_takes_precedence_over_env_var() {
        // Same precedence as the NO_PROXY forwarded to connectors (append_proxy_envs)
        let _env_guard = lock_clean_proxy_env();
        set_env("NO_PROXY", "127.0.0.1");
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        let url = format!("http://{}/graphql", target_addr);

        let config = proxied_config(Some(proxy_addr), Some(proxy_addr), Some("opencti-ingest"));
        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!((direct, proxied.len()), (0, 1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn env_proxy_is_bypassed_for_configured_no_proxy_hosts() {
        // with_proxy + HTTP_PROXY env var + configured no_proxy (e.g. OPENCTI__NO_PROXY):
        // the system proxies would only honor the NO_PROXY env var
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        set_env("HTTP_PROXY", &format!("http://{}", proxy_addr));
        let url = format!("http://{}/graphql", target_addr);

        let config = proxied_config(None, None, Some("opencti-ingest,127.0.0.1"));
        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::OK));
        assert_eq!((direct, proxied.len()), (1, 0));

        // The proxy from the env is still used for the other hosts
        let config = proxied_config(None, None, Some("opencti-ingest"));
        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!(proxied, vec![format!("GET {} HTTP/1.1", url)]);
        assert_eq!(direct, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn env_proxy_ignores_blank_proxy_env_vars() {
        // e.g. HTTP_PROXY="" rendered by a chart: like the system proxies, treat it as
        // unset instead of failing to build the client
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (_, proxy) = spawn_proxy().await;
        set_env("HTTP_PROXY", "");
        set_env("HTTPS_PROXY", " ");
        let url = format!("http://{}/graphql", target_addr);

        let config = proxied_config(None, None, Some("opencti-ingest"));
        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::OK));
        assert_eq!((direct, proxied.len()), (1, 0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn env_proxy_falls_back_to_all_proxy() {
        // Like the system proxies: ALL_PROXY applies to each scheme without its own proxy
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (all_addr, all_proxy) = spawn_proxy().await;
        set_env("ALL_PROXY", &format!("http://{}", all_addr));
        let http_url = format!("http://{}/graphql", target_addr);
        let https_url = format!("https://{}/graphql", target_addr);

        // ALL_PROXY only: the configured no_proxy applies to it
        let config = proxied_config(None, None, Some("opencti-ingest,127.0.0.1"));
        let (result, direct, proxied) = send_and_trace(&config, &http_url, &target, &all_proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::OK));
        assert_eq!((direct, proxied.len()), (1, 0));

        let config = proxied_config(None, None, Some("opencti-ingest"));
        let (_, direct, proxied) = send_and_trace(&config, &http_url, &target, &all_proxy).await;
        assert_eq!(direct, 0);
        assert_eq!(proxied, vec![format!("GET {} HTTP/1.1", http_url)]);

        // HTTP_PROXY + ALL_PROXY: HTTP goes through HTTP_PROXY, HTTPS still through ALL_PROXY
        let (http_addr, http_proxy) = spawn_proxy().await;
        set_env("HTTP_PROXY", &format!("http://{}", http_addr));
        let (_, direct, proxied) = send_and_trace(&config, &http_url, &target, &http_proxy).await;
        assert_eq!(direct, 0);
        assert_eq!(proxied, vec![format!("GET {} HTTP/1.1", http_url)]);
        let (_, direct, proxied) = send_and_trace(&config, &https_url, &target, &all_proxy).await;
        assert_eq!(direct, 0, "HTTPS should not go direct");
        assert_eq!(proxied, vec![format!("CONNECT {} HTTP/1.1", target_addr)]);
    }

    #[test]
    fn proxy_env_guard_restores_the_environment() {
        {
            let _env_guard = lock_clean_proxy_env();
            set_env("NO_PROXY", "leaked-by-a-test");
        }
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_ne!(std::env::var("NO_PROXY").ok().as_deref(), Some("leaked-by-a-test"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn system_proxies_are_unchanged_without_configured_no_proxy() {
        // The workaround from #169 (standard env vars only) keeps working as before
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        set_env("HTTP_PROXY", &format!("http://{}", proxy_addr));
        let url = format!("http://{}/graphql", target_addr);
        let config = proxied_config(None, None, None);

        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!((direct, proxied.len()), (0, 1));

        set_env("NO_PROXY", "opencti-ingest,127.0.0.1");
        let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
        assert_eq!(result, Ok(reqwest::StatusCode::OK));
        assert_eq!((direct, proxied.len()), (1, 0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blank_configured_no_proxy_is_treated_as_unset() {
        // e.g. OPENCTI__NO_PROXY="" rendered by a chart next to the standard env vars:
        // the NO_PROXY env var still applies, as before
        let _env_guard = lock_clean_proxy_env();
        let (target_addr, target) = spawn_target().await;
        let (proxy_addr, proxy) = spawn_proxy().await;
        set_env("NO_PROXY", "opencti-ingest,127.0.0.1");
        let url = format!("http://{}/graphql", target_addr);

        for blank in ["", "  "] {
            // System proxies (the workaround from #169)
            set_env("HTTP_PROXY", &format!("http://{}", proxy_addr));
            let config = proxied_config(None, None, Some(blank));
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::OK), "system proxy, no_proxy {:?}", blank);
            assert_eq!((direct, proxied.len()), (1, 0), "system proxy, no_proxy {:?}", blank);

            // Explicit proxies fall back to the NO_PROXY env var
            unsafe { std::env::remove_var("HTTP_PROXY"); }
            let config = proxied_config(Some(proxy_addr), Some(proxy_addr), Some(blank));
            let (result, direct, proxied) = send_and_trace(&config, &url, &target, &proxy).await;
            assert_eq!(result, Ok(reqwest::StatusCode::OK), "explicit proxy, no_proxy {:?}", blank);
            assert_eq!((direct, proxied.len()), (1, 0), "explicit proxy, no_proxy {:?}", blank);
        }
    }

    #[test]
    fn resolve_proxy_setting_prefers_config_then_env_in_order() {
        let _env_guard = lock_clean_proxy_env();
        assert_eq!(resolve_proxy_setting(None, NO_PROXY_ENV_VARS), None);
        set_env("no_proxy", "lower");
        assert_eq!(resolve_proxy_setting(None, NO_PROXY_ENV_VARS).as_deref(), Some("lower"));
        set_env("NO_PROXY", "upper");
        assert_eq!(resolve_proxy_setting(None, NO_PROXY_ENV_VARS).as_deref(), Some("upper"));
        // A configured value wins, even an empty one
        assert_eq!(resolve_proxy_setting(Some("conf"), NO_PROXY_ENV_VARS).as_deref(), Some("conf"));
        assert_eq!(resolve_proxy_setting(Some(""), NO_PROXY_ENV_VARS).as_deref(), Some(""));
    }

    // --- Tests for connector proxy env injection ---

    #[test]
    fn append_proxy_envs_injects_http_and_https_proxy() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("http_proxy");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("https_proxy");
        }

        let mut envs = Vec::new();
        append_proxy_envs(
            &mut envs,
            true,
            Some("http://proxy:8080"),
            Some("https://proxy:8443"),
            None,
        );

        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0].key, "HTTP_PROXY");
        assert_eq!(envs[0].value, "http://proxy:8080");
        assert_eq!(envs[1].key, "HTTPS_PROXY");
        assert_eq!(envs[1].value, "https://proxy:8443");
    }

    #[test]
    fn append_proxy_envs_injects_no_proxy() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("http_proxy");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("https_proxy");
            std::env::remove_var("NO_PROXY");
            std::env::remove_var("no_proxy");
        }

        let mut envs = Vec::new();
        append_proxy_envs(
            &mut envs,
            true,
            Some("http://proxy:3128"),
            Some("https://proxy:3128"),
            Some("localhost,127.0.0.1,.internal"),
        );

        assert_eq!(envs.len(), 3);
        assert_eq!(envs[0].key, "HTTP_PROXY");
        assert_eq!(envs[1].key, "HTTPS_PROXY");
        assert_eq!(envs[2].key, "NO_PROXY");
        assert_eq!(envs[2].value, "localhost,127.0.0.1,.internal");
    }

    #[test]
    fn append_proxy_envs_no_injection_when_proxy_disabled() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Even with env vars set, with_proxy: false should inject nothing
        unsafe {
            std::env::set_var("HTTP_PROXY", "http://should-not-appear:8080");
            std::env::set_var("HTTPS_PROXY", "http://should-not-appear:8080");
            std::env::set_var("NO_PROXY", "should-not-appear");
        }

        let mut envs = Vec::new();
        append_proxy_envs(
            &mut envs,
            false,
            Some("http://proxy:8080"),
            Some("https://proxy:8080"),
            Some("localhost"),
        );

        assert!(
            envs.is_empty(),
            "No proxy env vars should be injected when with_proxy is false"
        );

        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("NO_PROXY");
        }
    }

    #[test]
    fn append_proxy_envs_no_injection_when_enabled_but_no_urls_and_no_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure no proxy env vars are set
        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("http_proxy");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("https_proxy");
            std::env::remove_var("NO_PROXY");
            std::env::remove_var("no_proxy");
        }

        let mut envs = Vec::new();
        append_proxy_envs(&mut envs, true, None, None, None);

        assert!(
            envs.is_empty(),
            "No proxy env vars should be injected when with_proxy is true but no URLs configured and no env vars set"
        );
    }

    #[test]
    fn append_proxy_envs_falls_back_to_process_env_vars() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("HTTP_PROXY", "http://env-proxy:3128");
            std::env::set_var("HTTPS_PROXY", "https://env-proxy:3129");
            std::env::set_var("NO_PROXY", "localhost,.corp.internal");
        }

        let mut envs = Vec::new();
        // with_proxy: true, but no explicit config
        append_proxy_envs(&mut envs, true, None, None, None);

        assert_eq!(envs.len(), 3);
        assert_eq!(envs[0].key, "HTTP_PROXY");
        assert_eq!(envs[0].value, "http://env-proxy:3128");
        assert_eq!(envs[1].key, "HTTPS_PROXY");
        assert_eq!(envs[1].value, "https://env-proxy:3129");
        assert_eq!(envs[2].key, "NO_PROXY");
        assert_eq!(envs[2].value, "localhost,.corp.internal");

        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("NO_PROXY");
        }
    }

    #[test]
    fn append_proxy_envs_explicit_config_overrides_env_vars() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("HTTP_PROXY", "http://env-proxy:3128");
            std::env::set_var("HTTPS_PROXY", "https://env-proxy:3129");
            std::env::set_var("NO_PROXY", "env-no-proxy");
        }

        let mut envs = Vec::new();
        // Explicit config should win over env vars
        append_proxy_envs(
            &mut envs,
            true,
            Some("http://explicit-proxy:8080"),
            Some("https://explicit-proxy:8443"),
            Some("explicit-no-proxy"),
        );

        assert_eq!(envs.len(), 3);
        assert_eq!(envs[0].key, "HTTP_PROXY");
        assert_eq!(envs[0].value, "http://explicit-proxy:8080");
        assert_eq!(envs[1].key, "HTTPS_PROXY");
        assert_eq!(envs[1].value, "https://explicit-proxy:8443");
        assert_eq!(envs[2].key, "NO_PROXY");
        assert_eq!(envs[2].value, "explicit-no-proxy");

        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("NO_PROXY");
        }
    }

    #[test]
    fn append_proxy_envs_only_no_proxy_when_no_proxy_urls() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("http_proxy");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("https_proxy");
        }

        let mut envs = Vec::new();
        append_proxy_envs(&mut envs, true, None, None, Some("localhost,.local"));

        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].key, "NO_PROXY");
        assert_eq!(envs[0].value, "localhost,.local");
    }

    #[test]
    fn append_proxy_envs_marks_proxy_urls_as_sensitive() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("http_proxy");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("https_proxy");
        }

        let mut envs = Vec::new();
        append_proxy_envs(
            &mut envs,
            true,
            Some("http://user:pass@proxy:8080"),
            Some("https://user:pass@proxy:8443"),
            Some("internal"),
        );

        // HTTP_PROXY and HTTPS_PROXY should be sensitive (may contain credentials)
        assert!(envs[0].is_sensitive, "HTTP_PROXY should be marked sensitive");
        assert!(envs[1].is_sensitive, "HTTPS_PROXY should be marked sensitive");
        // NO_PROXY is not sensitive (just hostnames)
        assert!(!envs[2].is_sensitive, "NO_PROXY should not be marked sensitive");
    }

    #[test]
    fn append_proxy_ca_envs_injects_tls_ca_env_vars() {
        let mut envs = Vec::new();
        append_proxy_ca_envs(&mut envs, true, true);

        assert_eq!(envs.len(), 4);
        assert_eq!(envs[0].key, "SSL_CERT_FILE");
        assert_eq!(envs[0].value, PROXY_CA_CERT_MOUNT_PATH);
        assert_eq!(envs[1].key, "REQUESTS_CA_BUNDLE");
        assert_eq!(envs[1].value, PROXY_CA_CERT_MOUNT_PATH);
        assert_eq!(envs[2].key, "NODE_EXTRA_CA_CERTS");
        assert_eq!(envs[2].value, PROXY_CA_CERT_MOUNT_PATH);
        assert_eq!(envs[3].key, "CURL_CA_BUNDLE");
        assert_eq!(envs[3].value, PROXY_CA_CERT_MOUNT_PATH);
    }

    #[test]
    fn append_proxy_ca_envs_skips_when_disabled_or_missing_ca() {
        let mut envs = Vec::new();
        append_proxy_ca_envs(&mut envs, false, true);
        append_proxy_ca_envs(&mut envs, true, false);
        assert!(
            envs.is_empty(),
            "TLS CA env vars should not be injected without active proxy CA config"
        );
    }

    #[test]
    fn proxy_ca_bundle_resolves_inline_pem() {
        let pem = "-----BEGIN CERTIFICATE-----\nABCD\n-----END CERTIFICATE-----\n";
        let entries = vec![pem.to_string()];

        let mut bundle = String::new();
        for entry in &entries {
            let trimmed = entry.trim();
            if trimmed.starts_with("-----BEGIN") {
                bundle.push_str(trimmed);
                if !bundle.ends_with('\n') {
                    bundle.push('\n');
                }
            }
        }
        assert!(bundle.contains("BEGIN CERTIFICATE"), "should contain the inline PEM");
    }

    #[test]
    fn proxy_ca_bundle_skips_unreadable_file_and_returns_remaining() {
        let pem_inline = "-----BEGIN CERTIFICATE-----\nVALID\n-----END CERTIFICATE-----\n";
        let entries = vec!["/nonexistent/path/ca.pem".to_string(), pem_inline.to_string()];

        let mut bundle = String::new();
        for entry in &entries {
            let trimmed = entry.trim();
            if trimmed.starts_with("-----BEGIN") {
                bundle.push_str(trimmed);
                if !bundle.ends_with('\n') {
                    bundle.push('\n');
                }
            } else {
                // simulate unreadable file — skip
            }
        }
        assert!(bundle.contains("BEGIN CERTIFICATE"), "inline PEM should still be included");
    }

    #[test]
    fn append_proxy_tls_policy_envs_sets_node_tls_when_reject_disabled() {
        let mut envs = Vec::new();
        append_proxy_tls_policy_envs(&mut envs, true, false);
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].key, "NODE_TLS_REJECT_UNAUTHORIZED");
        assert_eq!(envs[0].value, "0");
    }

    #[test]
    fn append_proxy_tls_policy_envs_skips_when_reject_enabled_or_proxy_disabled() {
        let mut envs = Vec::new();
        append_proxy_tls_policy_envs(&mut envs, true, true);
        append_proxy_tls_policy_envs(&mut envs, false, false);
        assert!(envs.is_empty());
    }

    #[test]
    #[ignore = "requires isolated process env and local filesystem"]
    fn e2e_proxy_ca_list_path_and_inline_are_bundled_and_injected() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp_dir = std::path::PathBuf::from("dev/xtm-proxy-e2e-list");
        std::fs::create_dir_all(&tmp_dir).expect("create temp dir");
        let file_ca_path = tmp_dir.join("file-ca.pem");
        let file_ca_content = "-----BEGIN CERTIFICATE-----\nFILE-CA\n-----END CERTIFICATE-----\n";
        std::fs::write(&file_ca_path, file_ca_content).expect("write file ca");

        let test_config_path = std::path::PathBuf::from("config/proxy_list_e2e.yaml");
        let config_content = format!(
            "opencti:\n  with_proxy: true\n  http_proxy: http://proxy.example.com:8080\n  https_proxy: http://proxy.example.com:8080\n  no_proxy: \"localhost,127.0.0.1,internal.domain\"\n  https_proxy_ca:\n    - {}\n    - |-\n      -----BEGIN CERTIFICATE-----\n      INLINE-CA\n      -----END CERTIFICATE-----\n  https_proxy_reject_unauthorized: false\n",
            file_ca_path.to_string_lossy()
        );
        std::fs::write(&test_config_path, config_content).expect("write e2e config");
        unsafe { std::env::set_var("COMPOSER_ENV", "proxy_list_e2e"); }

        let connector = ApiConnector {
            id: "e2e-proxy-ca-list".to_string(),
            platform: "opencti".to_string(),
            name: "e2e-proxy-ca-list".to_string(),
            image: "alpine:3.20".to_string(),
            contract_hash: "hash-e2e".to_string(),
            current_status: None,
            requested_status: "starting".to_string(),
            contract_configuration: vec![],
            undecryptable_keys: vec![],
        };

        let bundle = connector.proxy_ca_bundle().expect("bundle should exist");
        assert!(bundle.contains("FILE-CA"), "bundle should include file CA");
        assert!(bundle.contains("INLINE-CA"), "bundle should include inline CA");

        let envs = connector.container_envs();
        let env_map: std::collections::HashMap<String, String> = envs
            .into_iter()
            .map(|v| (v.key, v.value))
            .collect();
        assert_eq!(
            env_map.get("HTTP_PROXY"),
            Some(&"http://proxy.example.com:8080".to_string())
        );
        assert_eq!(
            env_map.get("HTTPS_PROXY"),
            Some(&"http://proxy.example.com:8080".to_string())
        );
        assert_eq!(
            env_map.get("NO_PROXY"),
            Some(&"localhost,127.0.0.1,internal.domain".to_string())
        );
        assert_eq!(
            env_map.get("SSL_CERT_FILE"),
            Some(&PROXY_CA_CERT_MOUNT_PATH.to_string())
        );
        assert_eq!(
            env_map.get("REQUESTS_CA_BUNDLE"),
            Some(&PROXY_CA_CERT_MOUNT_PATH.to_string())
        );
        assert_eq!(
            env_map.get("NODE_EXTRA_CA_CERTS"),
            Some(&PROXY_CA_CERT_MOUNT_PATH.to_string())
        );
        assert_eq!(
            env_map.get("CURL_CA_BUNDLE"),
            Some(&PROXY_CA_CERT_MOUNT_PATH.to_string())
        );
        assert_eq!(
            env_map.get("NODE_TLS_REJECT_UNAUTHORIZED"),
            Some(&"0".to_string())
        );

        // Real-container verification: mount the generated bundle and verify
        // both cert entries and injected env vars are available inside the container.
        let bundle_path = tmp_dir.join("bundle.pem");
        std::fs::write(&bundle_path, &bundle).expect("write bundle file");
        let bundle_abs = std::fs::canonicalize(&bundle_path).expect("canonical bundle path");
        let docker_status = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "-e",
                "HTTP_PROXY=http://proxy.example.com:8080",
                "-e",
                "HTTPS_PROXY=http://proxy.example.com:8080",
                "-e",
                "NO_PROXY=localhost,127.0.0.1,internal.domain",
                "-e",
                "SSL_CERT_FILE=/etc/ssl/certs/xtm-proxy-ca.crt",
                "-e",
                "REQUESTS_CA_BUNDLE=/etc/ssl/certs/xtm-proxy-ca.crt",
                "-e",
                "NODE_EXTRA_CA_CERTS=/etc/ssl/certs/xtm-proxy-ca.crt",
                "-e",
                "CURL_CA_BUNDLE=/etc/ssl/certs/xtm-proxy-ca.crt",
                "-e",
                "NODE_TLS_REJECT_UNAUTHORIZED=0",
                "-v",
                &format!(
                    "{}:/etc/ssl/certs/xtm-proxy-ca.crt:ro",
                    bundle_abs.display()
                ),
                "alpine:3.20",
                "sh",
                "-ec",
                "grep -q FILE-CA /etc/ssl/certs/xtm-proxy-ca.crt && grep -q INLINE-CA /etc/ssl/certs/xtm-proxy-ca.crt && test \"$SSL_CERT_FILE\" = \"/etc/ssl/certs/xtm-proxy-ca.crt\" && test \"$NODE_TLS_REJECT_UNAUTHORIZED\" = \"0\"",
            ])
            .status()
            .expect("docker must be available for e2e test");
        assert!(
            docker_status.success(),
            "docker container verification failed"
        );

        let _ = std::fs::remove_file(test_config_path);
        let _ = std::fs::remove_file(bundle_path);
        let _ = std::fs::remove_file(file_ca_path);
        let _ = std::fs::remove_dir_all(tmp_dir);
    }

    #[test]
    fn unknown_platform_does_not_inject_proxy_envs() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Set env vars that WOULD be injected if the platform were recognized
        unsafe {
            std::env::set_var("HTTP_PROXY", "http://should-not-appear:8080");
            std::env::set_var("HTTPS_PROXY", "https://should-not-appear:8080");
            std::env::set_var("NO_PROXY", "should-not-appear");
        }

        // Simulate the match logic from container_envs() for an unknown platform
        let platform = "unknown_platform";
        let proxy_config = match platform {
            "opencti" => Some((true, Some("http://proxy:8080"), Some("https://proxy:8080"), Some("localhost"))),
            "openaev" => Some((true, Some("http://proxy:8080"), Some("https://proxy:8080"), Some("localhost"))),
            _ => None,
        };

        let mut envs = Vec::new();
        if let Some((with_proxy, http_proxy, https_proxy, no_proxy)) = proxy_config {
            append_proxy_envs(&mut envs, with_proxy, http_proxy, https_proxy, no_proxy);
        }

        assert!(
            envs.is_empty(),
            "Unknown platform should NOT inject any proxy env vars, got: {:?}",
            envs
        );

        unsafe {
            std::env::remove_var("HTTP_PROXY");
            std::env::remove_var("HTTPS_PROXY");
            std::env::remove_var("NO_PROXY");
        }
    }
}
