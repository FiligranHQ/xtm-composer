use bollard::Docker;
use crate::config::settings::Swarm;

pub mod swarm;

pub struct SwarmOrchestrator {
    docker: Docker,
    config: Swarm,
}
