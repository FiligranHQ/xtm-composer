use crate::config::settings::Daemon;
use bollard::Docker;

pub mod docker;

pub struct DockerOrchestrator {
    docker: Docker,
    daemon: Daemon,
}
