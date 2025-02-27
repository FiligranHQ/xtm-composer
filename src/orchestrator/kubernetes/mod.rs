pub mod kubernetes;

#[derive(Default)]
pub struct KubeOrchestrator {
    base_uri: String,
}