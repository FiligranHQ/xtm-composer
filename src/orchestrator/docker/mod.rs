use bollard::Docker;

pub mod docker;

pub struct DockerOrchestrator {
    pub docker: Docker,
}
