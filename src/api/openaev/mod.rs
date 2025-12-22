mod connector;
mod manager;
mod api_handler;

use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;
use async_trait::async_trait;
use std::time::Duration;
use rsa::RsaPrivateKey;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

pub struct ApiOpenAEV {
    api_uri: String,
    http_client: reqwest::Client,
    bearer: String,
    daemon: Daemon,
    logs_schedule: u64,
    private_key: RsaPrivateKey,
    // TODO: Implement timeout configuration when OpenBAS API methods are implemented
    // These fields are stored for future use when the todo!() macros are replaced with actual implementations
    #[allow(dead_code)]
    request_timeout: u64,
    #[allow(dead_code)]
    connect_timeout: u64,
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

        let http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(connect_timeout))
            .timeout(Duration::from_secs(request_timeout))
            .build()
            .unwrap(); // or handle the error appropriately

        let private_key = crate::private_key().clone();

        Self {
            api_uri,
            http_client,
            bearer,
            daemon,
            logs_schedule,
            private_key,
            request_timeout,
            connect_timeout,
        }
    }

    pub fn post(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri.clone(), route);

        self.http_client
            .post(&api_route)
            .header("Content-Type", "application/json")
            .header(AUTHORIZATION_HEADER, self.bearer.as_str())
    }

    pub fn put(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri.clone(), route);

        self.http_client
            .put(&api_route)
            .header("Content-Type", "application/json")
            .header(AUTHORIZATION_HEADER, self.bearer.as_str())
    }

    pub fn get(&self, route: &str) -> reqwest::RequestBuilder {
        let api_route = format!("{}{}", self.api_uri.clone(), route);

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
