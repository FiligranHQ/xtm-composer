use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;
use async_trait::async_trait;
use cynic::Operation;
use cynic::http::CynicReqwestError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use rsa::RsaPrivateKey;

pub mod connector;
pub mod manager;
pub mod error_handler;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

#[cynic::schema("opencti")]
pub mod opencti {}

pub struct ApiOpenCTI {
    api_uri: String,
    bearer: String,
    daemon: Daemon,
    logs_schedule: u64,
    request_timeout: u64,
    connect_timeout: u64,
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
        Self {
            api_uri,
            bearer,
            daemon,
            logs_schedule,
            request_timeout,
            connect_timeout,
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
        reqwest::Client::builder()
            .timeout(Duration::from_secs(self.request_timeout))
            .connect_timeout(Duration::from_secs(self.connect_timeout))
            .build()
            .unwrap()
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

    async fn container_removed_successfully(&self, id: String) -> () {
        // Do nothing
    }
}
