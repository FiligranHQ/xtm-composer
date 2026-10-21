use crate::config::settings::{Docker as DockerOptions, Registry};
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
}
