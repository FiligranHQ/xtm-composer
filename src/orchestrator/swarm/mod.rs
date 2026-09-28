use bollard::Docker;
use crate::config::settings::{Registry, Swarm};

pub mod swarm;

pub struct SwarmOrchestrator {
    docker: Docker,
    config: Swarm,
    registry: Option<Registry>,
}
