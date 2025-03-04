// TODO Remove macro after implementation
#![allow(unused_variables)]

use crate::api::opencti::connector::ConnectorCurrentStatus;
use crate::api::ApiConnector;
use crate::config::settings::Settings;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use async_trait::async_trait;
use bollard::container::ListContainersOptions;
use bollard::Docker;
use tracing::error;
use std::collections::HashMap;

impl DockerOrchestrator {
    pub fn new() -> Self {
        let docker = Docker::connect_with_socket_defaults().unwrap();
        Self { docker }
    }
}

/*
async fn docker_handling() {
    use futures::TryStreamExt;

    info!("01. Connecting to docker socket");


    let version = docker.version().await.unwrap();
    info!("{:?}", version.version);

    // let mut list_container_filters = HashMap::new();
    // list_container_filters.insert("status", vec!["running"]);
    info!("02. Getting containers");
    let containers = docker.list_containers(Some(ListContainersOptions::<String> {
        all: true,
        // filters: list_container_filters,
        ..Default::default()
    })).await.unwrap();

    for _container in containers {
        // debug!("{:?} -> {:?} -- {:?}", container.id, container.state, container.labels);
    }

    info!("03. Create image / pulling");
    let _ = docker.create_image(Some(CreateImageOptions {
        from_image: "opencti/connector-ipinfo:latest",
        ..Default::default()
    }), None, None).try_for_each(|info| {
        info!("opencti/connector-ipinfo {:?} {:?} pulling...", info.status.as_deref(), info.progress.as_deref());
        future::ok(())
    }).await.unwrap();


    // Test create
    let container_name = "my-container";
    let existing_container = docker.inspect_container(container_name, None).await;
    match existing_container {
        Ok(_image) => {
            info!("04. Container {} already exists, removing ...", container_name);
            docker.remove_container(container_name, Some(RemoveContainerOptions {
                v: true,
                force: true,
                link: false
            })).await.unwrap();
        }
        Err(_err) => {
            info!("04. Container {} doest not exists, creating ...", container_name);
        }
    }

    let connector_id = "1445c7fd-42bf-466a-b4e6-d1b24fcca66d";
    let connector_contract = HashMap::from([
        ("OPENCTI_URL", "http://localhost:4000"),
        ("OPENCTI_TOKEN", "d434ce02-e58e-4cac-8b4c-42bf16748e84"),
        ("CONNECTOR_ID", connector_id),
        ("CONNECTOR_TYPE", "INTERNAL_ENRICHMENT"),
        ("CONNECTOR_NAME", "IpInfo"),
        ("CONNECTOR_SCOPE", "IPv4-Addr"),
        ("CONNECTOR_AUTO", "true"),
        ("IPINFO_TOKEN", "4f0b8a3ffc13d8"),
        ("IPINFO_MAX_TLP", "TLP:AMBER"),
        ("IPINFO_USE_ASN_NAME", "false"),
    ]);

    // let mut vec = Vec::new();
    // vec.append()

    let container_env_variables = connector_contract
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<String>>();
    let connector_env_config = container_env_variables
        .iter()
        .map(|t| t.as_str()).collect();

    let connector_config = Config {
        image: Some("opencti/connector-ipinfo"),
        env: Some(connector_env_config), // Contrat to env
        labels: Some(HashMap::from([("opencti-connector-id", connector_id)])),
        ..Default::default()
    };

    let _created_connector = docker
        .create_container::<&str, &str>(Some(CreateContainerOptions {
            name: container_name,
            platform: None,
        }), connector_config)
        .await.unwrap();

    info!("05. Starting the connector");
    //
    // docker.start_container(connector_config.i, None::<StartContainerOptions<String>>).unwrap()
    // docker.stop_container()
    // docker.kill_container()
}
*/

#[async_trait]
impl Orchestrator for DockerOrchestrator {

    async fn get(&self, connector: &ApiConnector) -> Option<OrchestratorContainer> {
        None
    }

    async fn list(&self, settings: &Settings) -> Option<Vec<OrchestratorContainer>> {
        let list_container_filters: HashMap<String, Vec<String>> = HashMap::from([
            ("opencti-manager".into(), Vec::from([settings.manager.id.clone()]))
        ]);
        let container_result = self
            .docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                filters: list_container_filters,
                ..Default::default()
            }))
            .await;
        match container_result {
            Ok(containers) => Some(
                containers
                    .into_iter()
                    .map(|docker_container| OrchestratorContainer {
                        id: docker_container.id.unwrap(),
                        state: docker_container.state.unwrap(),
                        // image: docker_container.image.unwrap(),
                        envs: HashMap::new(),
                        labels: docker_container.labels.unwrap(),
                    })
                    .collect(),
            ),
            Err(err) => {
                error!(error = err.to_string(), "Error fetching containers");
                None
            }
        }
    }

    async fn start(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        todo!("docker start")
    }

    async fn stop(&self, container: &OrchestratorContainer, connector: &ApiConnector) -> () {
        todo!("docker stop")
    }

    async fn refresh(
        &self,
        settings: &Settings,
        connector: &ApiConnector,
    ) -> Option<OrchestratorContainer> {
        todo!("docker refresh")
    }

    async fn remove(&self, container: &OrchestratorContainer) -> () {
        todo!("docker remove")
    }

    async fn deploy(
        &self,
        settings: &Settings,
        connector: &ApiConnector,
    ) -> Option<OrchestratorContainer> {
        todo!("docker deploy")
    }

    async fn logs(
        &self,
        container: &OrchestratorContainer,
        connector: &ApiConnector,
    ) -> Option<Vec<String>> {
        todo!("docker logs")
    }

    fn state_converter(&self, container: &OrchestratorContainer) -> ConnectorCurrentStatus {
        todo!("docker state_converter")
    }
}
