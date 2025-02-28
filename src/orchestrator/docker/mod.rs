use bollard::Docker;

pub mod docker;

pub struct DockerOrchestrator {
    docker: Docker,
}
