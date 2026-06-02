mod connector;
mod manager;
mod api_handler;

use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;
use async_trait::async_trait;
use std::time::Duration;
use rsa::RsaPrivateKey;
use tracing::info;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

pub struct ApiOpenAEV {
    api_uri: String,
    http_client: reqwest::Client,
    bearer: String,
    daemon: Daemon,
    logs_schedule: u64,
    private_key: RsaPrivateKey,
}

impl ApiOpenAEV {
    pub fn new() -> Self {
        let settings = crate::settings();
        let bearer = format!("{} {}", BEARER, settings.openaev.token);
        let api_uri = format!("{}/api", &settings.openaev.url);
        let daemon = settings.openaev.daemon.clone();
        let logs_schedule = settings.openaev.logs_schedule;
        let request_timeout = settings.openaev.request_timeout;
        let connect_timeout = settings.openaev.connect_timeout;

        // Build HTTP client with proxy and TLS settings
        let mut client_builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(connect_timeout))
            .timeout(Duration::from_secs(request_timeout))
            .danger_accept_invalid_certs(settings.openaev.unsecured_certificate);

        if settings.openaev.with_proxy {
            if let Some(proxy_url) = &settings.openaev.proxy_url {
                info!(proxy_url = proxy_url.as_str(), "OpenAEV using explicit proxy");
                let proxy = reqwest::Proxy::all(proxy_url)
                    .expect("Invalid proxy URL in openaev.proxy_url");
                client_builder = client_builder.proxy(proxy);
            }
            // If with_proxy is true but no proxy_url, reqwest uses system proxies by default
        } else {
            // Disable all proxy usage (ignore system env vars)
            client_builder = client_builder.no_proxy();
        }

        let http_client = client_builder.build().unwrap();

        let private_key = crate::private_key().clone();

        Self {
            api_uri,
            http_client,
            bearer,
            daemon,
            logs_schedule,
            private_key,
        }
    }

    pub fn post(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri, route);

        self.http_client
            .post(&api_route)
            .header("Content-Type", "application/json")
            .header(AUTHORIZATION_HEADER, self.bearer.as_str())
    }

    pub fn put(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri, route);

        self.http_client
            .put(&api_route)
            .header("Content-Type", "application/json")
            .header(AUTHORIZATION_HEADER, self.bearer.as_str())
    }

    pub fn get(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri, route);

        self.http_client
            .get(&api_route)
            .header(AUTHORIZATION_HEADER, self.bearer.as_str())
    }

}

#[async_trait]
impl ComposerApi for ApiOpenAEV {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    fn platform(&self) -> &'static str {
        "openaev"
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(self.logs_schedule)
    }

    async fn version(&self) -> Option<String> {
        manager::get_version::get_version(self).await
    }

    async fn ping_alive(&self) -> Option<String> {
        manager::ping_alive::ping_alive(self).await
    }

    async fn register(&self) {
        manager::post_register::register(self).await
    }

    async fn connectors(&self) -> Option<Vec<ApiConnector>> {
        connector::get_connector_instances::get_connector_instances(self).await
    }

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector> {
        connector::patch_status::update_status(id, status, self).await
    }

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String> {
        connector::post_logs::add_logs(id, logs, self).await
    }

    async fn patch_health(&self, id: String, restart_count: u32, started_at: String, is_in_reboot_loop: bool) -> Option<String> {
        connector::patch_health::update_health(id, restart_count, started_at, is_in_reboot_loop, self).await
    }
}
