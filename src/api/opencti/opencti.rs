use crate::api::opencti::connector::ConnectorCurrentStatus;
use crate::api::{ApiConnector, ComposerApi};
use crate::config::settings::{Daemon, Settings};
use async_trait::async_trait;
use cynic::Operation;
use serde::de::DeserializeOwned;
use serde::Serialize;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

pub struct ApiOpenCTI {
    api_uri: String,
    bearer: String,
    daemon: Daemon,
}

impl ApiOpenCTI {
    pub fn new(settings: &Settings) -> Self {
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
    
    async fn register(&self, settings: &Settings) -> Option<String> {
        crate::api::opencti::manager::register_manager(settings, self).await
    }

    async fn connectors(&self, settings: &Settings) -> Option<Vec<ApiConnector>> {
        crate::api::opencti::connector::list(settings, self).await
    }

    async fn patch_status(
        &self,
        connector_id: String,
        status: ConnectorCurrentStatus,
    ) -> Option<ApiConnector> {
        crate::api::opencti::connector::patch_status(connector_id, status, self).await
    }

    async fn patch_logs(&self, connector_id: String, logs: Vec<String>) -> Option<ApiConnector> {
        crate::api::opencti::connector::patch_logs(connector_id, logs, self).await
    }
}
