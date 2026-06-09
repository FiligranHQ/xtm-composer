use crate::config::settings::Daemon;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

pub mod openaev;
pub mod opencti;
mod decrypt_value;

pub const PROXY_CA_CERT_MOUNT_PATH: &str = "/etc/ssl/certs/xtm-proxy-ca.crt";

#[derive(Debug, Clone)]
struct PlatformProxyConfig {
    with_proxy: bool,
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    no_proxy: Option<String>,
    https_proxy_ca: Option<String>,
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
    pub platform_name: String,
}

/// Build a reqwest HTTP client configured with proxy and TLS settings.
///
/// - `with_proxy: false` → disables all proxies (ignores system env vars).
/// - `with_proxy: true` + explicit `http_proxy`/`https_proxy` → uses configured proxies.
/// - `with_proxy: true` + no explicit proxy → uses system proxies (HTTP_PROXY/HTTPS_PROXY env vars).
pub fn build_http_client(config: &HttpClientConfig) -> Result<reqwest::Client, reqwest::Error> {
    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.request_timeout))
        .connect_timeout(Duration::from_secs(config.connect_timeout))
        .danger_accept_invalid_certs(config.unsecured_certificate);

    if config.with_proxy {
        if let Some(http_proxy) = &config.http_proxy {
            info!(platform = %config.platform_name, "Using explicit HTTP proxy");
            let proxy = reqwest::Proxy::http(http_proxy)
                .unwrap_or_else(|e| panic!("Invalid http_proxy for platform '{}': {}", config.platform_name, e));
            client_builder = client_builder.proxy(proxy);
        }
        if let Some(https_proxy) = &config.https_proxy {
            info!(platform = %config.platform_name, "Using explicit HTTPS proxy");
            let proxy = reqwest::Proxy::https(https_proxy)
                .unwrap_or_else(|e| panic!("Invalid https_proxy for platform '{}': {}", config.platform_name, e));
            client_builder = client_builder.proxy(proxy);
        }
        // If with_proxy is true but no explicit proxies, reqwest uses system proxies by default
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

    // Resolve HTTP_PROXY: explicit config wins, else inherit from process env
    let resolved_http = http_proxy
        .map(|s| s.to_string())
        .or_else(|| std::env::var("HTTP_PROXY").ok())
        .or_else(|| std::env::var("http_proxy").ok());

    // Resolve HTTPS_PROXY: explicit config wins, else inherit from process env
    let resolved_https = https_proxy
        .map(|s| s.to_string())
        .or_else(|| std::env::var("HTTPS_PROXY").ok())
        .or_else(|| std::env::var("https_proxy").ok());

    // Resolve NO_PROXY: explicit config wins, else inherit from process env
    let resolved_no_proxy = no_proxy
        .map(|s| s.to_string())
        .or_else(|| std::env::var("NO_PROXY").ok())
        .or_else(|| std::env::var("no_proxy").ok());

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

    /// Return the configured host file path for the HTTPS proxy CA certificate.
    pub fn proxy_ca_host_filepath(&self) -> Option<String> {
        let config = self.platform_proxy_config()?;
        if !config.with_proxy {
            return None;
        }
        config.https_proxy_ca
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
                proxy_config.https_proxy_ca.is_some(),
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
    use std::sync::Mutex;
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

        let _env_guard = ENV_LOCK.lock().unwrap();
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

    // --- Tests for connector proxy env injection ---

    #[test]
    fn append_proxy_envs_injects_http_and_https_proxy() {
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
        let _guard = ENV_LOCK.lock().unwrap();
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
    fn unknown_platform_does_not_inject_proxy_envs() {
        let _guard = ENV_LOCK.lock().unwrap();
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
