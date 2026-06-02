use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;
use async_trait::async_trait;
use cynic::Operation;
use cynic::http::CynicReqwestError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use rsa::RsaPrivateKey;
use tracing::info;

pub mod connector;
pub mod manager;
pub mod error_handler;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

#[cynic::schema("opencti")]
pub mod opencti {}

pub struct ApiOpenCTI {
    api_uri: String,
    http_client: reqwest::Client,
    bearer: String,
    daemon: Daemon,
    logs_schedule: u64,
    private_key: RsaPrivateKey,
}

impl ApiOpenCTI {
    pub fn new() -> Self {
        let settings = crate::settings();
        let bearer = format!("{} {}", BEARER, settings.opencti.token);
        let api_uri = format!("{}/graphql", &settings.opencti.url);
        let daemon = settings.opencti.daemon.clone();
        let logs_schedule = settings.opencti.logs_schedule;
        let request_timeout = settings.opencti.request_timeout;
        let connect_timeout = settings.opencti.connect_timeout;
        // Use the singleton private key
        let private_key = crate::private_key().clone();

        // Build HTTP client with proxy and TLS settings
        let mut client_builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(request_timeout))
            .connect_timeout(Duration::from_secs(connect_timeout))
            .danger_accept_invalid_certs(settings.opencti.unsecured_certificate);

        if settings.opencti.with_proxy {
            if let Some(proxy_url) = &settings.opencti.proxy_url {
                info!(proxy_url = proxy_url.as_str(), "OpenCTI using explicit proxy");
                let proxy = reqwest::Proxy::all(proxy_url)
                    .expect("Invalid proxy URL in opencti.proxy_url");
                client_builder = client_builder.proxy(proxy);
            }
            // If with_proxy is true but no proxy_url, reqwest uses system proxies by default
        } else {
            // Disable all proxy usage (ignore system env vars)
            client_builder = client_builder.no_proxy();
        }

        let http_client = client_builder.build().unwrap();

        Self {
            api_uri,
            http_client,
            bearer,
            daemon,
            logs_schedule,
            private_key
        }
    }

    pub async fn query_fetch<R, V>(
        &self,
        query: Operation<R, V>,
    ) -> Result<cynic::GraphQlResponse<R>, CynicReqwestError>
    where
        V: Serialize,
        R: DeserializeOwned + 'static,
    {
        use cynic::http::ReqwestExt;
        self.http_client
            .post(self.api_uri.clone())
            .header(AUTHORIZATION_HEADER, self.bearer.clone().as_str())
            .run_graphql(query)
            .await
    }
}

#[async_trait]
impl ComposerApi for ApiOpenCTI {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    fn platform(&self) -> &'static str {
        "opencti"
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(self.logs_schedule)
    }

    async fn version(&self) -> Option<String> {
        manager::get_version::version(self).await
    }

    async fn ping_alive(&self) -> Option<String> {
        manager::post_ping::ping(self).await
    }

    async fn register(&self) {
        manager::post_register::register(self).await
    }

    async fn connectors(&self) -> Option<Vec<ApiConnector>> {
        connector::get_listing::list(self).await
    }

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector> {
        connector::post_status::status(id, status, self).await
    }

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<String> {
        connector::post_logs::logs(id, logs, self).await
    }

    async fn patch_health(&self, id: String, restart_count: u32, started_at: String, is_in_reboot_loop: bool) -> Option<String> {
        connector::post_health::health(id, restart_count, started_at, is_in_reboot_loop, self).await
    }
}
