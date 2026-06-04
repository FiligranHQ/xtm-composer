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

/// Configuration for building an HTTP client with proxy and TLS settings.
pub struct HttpClientConfig {
    pub request_timeout: u64,
    pub connect_timeout: u64,
    pub unsecured_certificate: bool,
    pub with_proxy: bool,
    pub proxy_url: Option<String>,
    pub platform_name: &'static str,
}

/// Build a reqwest HTTP client configured with proxy and TLS settings.
///
/// - `with_proxy: false` → disables all proxies (ignores system env vars).
/// - `with_proxy: true` + `proxy_url: Some(url)` → uses the explicit proxy.
/// - `with_proxy: true` + `proxy_url: None` → uses system proxies (HTTP_PROXY/HTTPS_PROXY).
pub fn build_http_client(config: &HttpClientConfig) -> reqwest::Client {
    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.request_timeout))
        .connect_timeout(Duration::from_secs(config.connect_timeout))
        .danger_accept_invalid_certs(config.unsecured_certificate);

    if config.with_proxy {
        if let Some(proxy_url) = &config.proxy_url {
            info!(platform = config.platform_name, "Using explicit proxy");
            let proxy = reqwest::Proxy::all(proxy_url)
                .expect("Invalid proxy URL configuration");
            client_builder = client_builder.proxy(proxy);
        }
        // If with_proxy is true but no proxy_url, reqwest uses system proxies by default
    } else {
        // Disable all proxy usage (ignore system env vars)
        client_builder = client_builder.no_proxy();
    }

    client_builder.build().unwrap()
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

impl ApiConnector {
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
            proxy_url: None,
            platform_name: "test",
        }
    }

    #[test]
    fn build_client_with_proxy_disabled() {
        let config = HttpClientConfig {
            with_proxy: false,
            proxy_url: None,
            ..base_config()
        };
        let client = build_http_client(&config);
        // Client builds successfully with no proxy
        drop(client);
    }

    #[test]
    fn build_client_with_proxy_enabled_no_url() {
        let config = HttpClientConfig {
            with_proxy: true,
            proxy_url: None,
            ..base_config()
        };
        let client = build_http_client(&config);
        // Client builds successfully using system proxies
        drop(client);
    }

    #[test]
    fn build_client_with_explicit_proxy_url() {
        let config = HttpClientConfig {
            with_proxy: true,
            proxy_url: Some("http://127.0.0.1:9999".to_string()),
            ..base_config()
        };
        let client = build_http_client(&config);
        // Client builds successfully with explicit proxy
        drop(client);
    }

    #[test]
    fn build_client_with_unsecured_certificate() {
        let config = HttpClientConfig {
            unsecured_certificate: true,
            ..base_config()
        };
        let client = build_http_client(&config);
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
                proxy_url: Some(url.to_string()),
                ..base_config()
            };
            let client = build_http_client(&config);
            drop(client);
        }
    }

    #[tokio::test]
    async fn proxy_enabled_with_unreachable_proxy_fails_request() {
        // Configure proxy to an address where nothing listens
        let config = HttpClientConfig {
            with_proxy: true,
            proxy_url: Some("http://127.0.0.1:1".to_string()),
            request_timeout: 1,
            connect_timeout: 1,
            ..base_config()
        };
        let client = build_http_client(&config);

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
            proxy_url: Some(format!("http://{}", proxy_addr)),
            request_timeout: 2,
            connect_timeout: 2,
            ..base_config()
        };
        let client = build_http_client(&config);

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
            proxy_url: None,
            request_timeout: 1,
            connect_timeout: 1,
            ..base_config()
        };
        let client = build_http_client(&config);

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
}

