use crate::config::settings::{Docker as DockerOptions, Registry};
use crate::orchestrator::LogHistory;
use bollard::Docker;
use compat::CompatApi;

pub mod compat;
pub mod docker;
#[cfg(test)]
mod integration_tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContainerEngine {
    Docker,
    Podman,
}

pub struct DockerOrchestrator {
    docker: Docker,
    engine: ContainerEngine,
    /// Always set for Podman. Unset only on hosts without Unix sockets.
    compat: Option<CompatApi>,
    options: Option<DockerOptions>,
    registry: Option<Registry>,
    /// Last lines of each workload's previous container, kept across a recreation
    log_history: LogHistory,
}
