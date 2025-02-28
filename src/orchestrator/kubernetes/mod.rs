use kube::Client;

pub mod kubernetes;

pub struct KubeOrchestrator {
    base_uri: String,
    client: Client,
}
