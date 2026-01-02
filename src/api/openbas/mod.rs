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
    // TODO: Implement timeout configuration when OpenBAS API methods are implemented
    // These fields are stored for future use when the todo!() macros are replaced with actual implementations
    #[allow(dead_code)]
    request_timeout: u64,
    #[allow(dead_code)]
    connect_timeout: u64,
}

impl ApiOpenBAS {
    pub fn new() -> Self {
        let settings = &crate::config::settings::SETTINGS;
        let bearer = format!("{} {}", BEARER, settings.openbas.token);
        let api_uri = format!("{}/api", &settings.openbas.url);
        let daemon = settings.openbas.daemon.clone();
        let logs_schedule = settings.openbas.logs_schedule;
        let request_timeout = settings.openbas.request_timeout;
        let connect_timeout = settings.openbas.connect_timeout;
        Self {
            api_uri,
            bearer,
            daemon,
            logs_schedule,
            request_timeout,
            connect_timeout,
        }
    }
}

#[async_trait]
impl ComposerApi for ApiOpenBAS {
    fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    fn post_logs_schedule(&self) -> Duration {
        Duration::from_secs(self.logs_schedule)
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

    async fn patch_logs(&self, id: String, logs: Vec<String>) -> Option<cynic::Id> {
        todo!()
    }

    async fn patch_health(
        &self,
        id: String,
        restart_count: u32,
        started_at: String,
        is_in_reboot_loop: bool,
    ) -> Option<cynic::Id> {
        todo!()
    }
}
