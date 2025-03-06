use async_trait::async_trait;
use cynic::Operation;
use serde::de::DeserializeOwned;
use serde::Serialize;
use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;

pub mod manager;
pub mod connector;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

#[cynic::schema("opencti")]
pub mod opencti {}

pub struct ApiOpenCTI {
    api_uri: String,
    bearer: String,
    daemon: Daemon,
}

impl ApiOpenCTI {
    pub fn new() -> Self {
        let settings = crate::settings();
        let bearer = format!("{} {}", BEARER, settings.opencti.token);
        let api_uri = format!("{}/graphql", &settings.opencti.url);
        let daemon = settings.opencti.daemon.clone();
        Self { api_uri, bearer, daemon }
    }

    pub async fn query_fetch<R, V>(&self, query: Operation<R, V>) -> cynic::GraphQlResponse<R>
    where
        V: Serialize,
        R: DeserializeOwned + 'static,
    {
        use cynic::http::ReqwestExt;
        reqwest::Client::builder()
            .build()
            .unwrap()
            .post(self.api_uri.clone())
            .header(AUTHORIZATION_HEADER, self.bearer.clone().as_str())
            .run_graphql(query)
            .await
            .unwrap()
    }
}

#[async_trait]
impl ComposerApi for ApiOpenCTI {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    async fn ping_alive(&self) -> () {
        crate::api::opencti::manager::post_ping::ping_alive(self).await
    }

    async fn register(&self) -> Option<String> {
        crate::api::opencti::manager::post_register::register_manager(self).await
    }

    async fn connectors(&self) -> Option<Vec<ApiConnector>> {
        crate::api::opencti::connector::get_listing::list(self).await
    }

    async fn patch_status(
        &self,
        connector_id: String,
        status: ConnectorStatus,
    ) -> Option<ApiConnector> {
        crate::api::opencti::connector::post_status::patch_status(connector_id, status, self).await
    }

    async fn patch_logs(&self, connector_id: String, logs: Vec<String>) -> Option<ApiConnector> {
        crate::api::opencti::connector::post_logs::patch_logs(connector_id, logs, self).await
    }
}
