// TODO Remove macro after implementation
#![allow(unused_variables)]

use crate::api::{ApiConnector, ComposerApi, ConnectorStatus};
use crate::config::settings::Daemon;
use async_trait::async_trait;
use std::time::Duration;
use tracing::debug;

const BEARER: &str = "Bearer";

pub struct ApiOpenBAS {
    api_uri: String,
    bearer: String,
    daemon: Daemon,
    logs_schedule: u64,
}

impl ApiOpenBAS {
    pub fn new() -> Self {
        let settings = crate::settings();
        let bearer = format!("{} {}", BEARER, settings.openbas.token);
        let api_uri = format!("{}/api", &settings.openbas.url);
        let daemon = settings.openbas.daemon.clone();
        let logs_schedule = settings.openbas.logs_schedule;
        Self {
            api_uri,
            bearer,
            daemon,
            logs_schedule,
        }
    }
}

#[async_trait]
impl ComposerApi for ApiOpenBAS {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(self.logs_schedule * 60)
    }

    async fn version(&self) -> Option<String> {
        todo!()
    }

    async fn ping_alive(&self) -> Option<String> {
        todo!()
    }

    async fn register(&self) {
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

    async fn patch_status(&self, id: String, status: ConnectorStatus) -> Option<ApiConnector> {
        todo!()
    }

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<ApiConnector> {
        todo!()
    }
}
