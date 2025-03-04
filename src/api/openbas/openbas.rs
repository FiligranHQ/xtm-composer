// TODO Remove macro after implementation
#![allow(unused_variables)]

use crate::api::opencti::connector::ConnectorCurrentStatus;
use crate::api::{ApiConnector, ComposerApi};
use crate::config::settings::Settings;
use async_trait::async_trait;
use log::info;

const BEARER: &str = "Bearer";

pub struct ApiOpenBAS {
    api_uri: String,
    bearer: String,
}

impl ApiOpenBAS {
    pub fn new(settings: &Settings) -> Self {
        let bearer = format!("{} {}", BEARER, settings.openbas.token);
        let api_uri = format!("{}/api", &settings.openbas.url);
        Self { api_uri, bearer }
    }
}

#[async_trait]
impl ComposerApi for ApiOpenBAS {
    async fn register(&self, settings: &Settings) -> Option<String> {
        info!("{} {}", self.api_uri, self.bearer);
        todo!()
    }

    async fn connectors(&self, settings: &Settings) -> Option<Vec<ApiConnector>> {
        todo!()
    }

    async fn patch_status(
        &self,
        connector_id: String,
        status: ConnectorCurrentStatus,
    ) -> Option<ApiConnector> {
        todo!()
    }

    async fn patch_logs(&self, connector_id: String, logs: Vec<String>) -> Option<ApiConnector> {
        todo!()
    }
}
