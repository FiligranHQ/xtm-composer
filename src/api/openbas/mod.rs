// TODO Remove macro after implementation
#![allow(unused_variables)]

use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::{Daemon};
use async_trait::async_trait;
use tracing::debug;

const BEARER: &str = "Bearer";

pub struct ApiOpenBAS {
    api_uri: String,
    bearer: String,
    daemon: Daemon,
}

impl ApiOpenBAS {
    pub fn new() -> Self {
        let settings = crate::settings();
        let bearer = format!("{} {}", BEARER, settings.openbas.token);
        let api_uri = format!("{}/api", &settings.openbas.url);
        let daemon = settings.openbas.daemon.clone();
        Self {
            api_uri,
            bearer,
            daemon,
        }
    }
}

#[async_trait]
impl ComposerApi for ApiOpenBAS {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    async fn ping_alive(&self) -> () {
        todo!()
    }

    async fn register(&self) -> Option<String> {
        debug!(
            api_uri = self.api_uri,
            bearer = self.bearer,
            "OpenBAS register"
        );
        todo!()
    }

    async fn connectors(&self) -> Option<Vec<ApiConnector>> {
        todo!()
    }

    async fn patch_status(
        &self,
        connector_id: String,
        status: ConnectorStatus,
    ) -> Option<ApiConnector> {
        todo!()
    }

    async fn patch_logs(&self, connector_id: String, logs: Vec<String>) -> Option<ApiConnector> {
        todo!()
    }
}
