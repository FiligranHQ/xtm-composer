use std::collections::HashMap;
use async_trait::async_trait;
use header::HeaderValue;
use log::{error};
use reqwest::{header, Client};
use reqwest::header::HeaderMap;
use crate::api::connector::{Connector};
use crate::config::settings::Portainer;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use crate::orchestrator::portainer::{PortainerDeployHostConfig, PortainerDeployPayload, PortainerDeployResponse, PortainerGetResponse, PortainerOrchestrator};

const X_API_KEY: &str = "X-API-KEY";

impl PortainerOrchestrator {
    pub fn new(config: &Portainer) -> Self {
        let container_uri = format!("{}/api/endpoints/{}/docker/{}/containers",
                               config.api, config.env_id, config.api_version);
        let image_uri = format!("{}/api/endpoints/{}/docker/{}/images",
                               config.api, config.env_id, config.api_version);
        let mut headers = HeaderMap::new();
        headers.insert(X_API_KEY, HeaderValue::from_bytes(config.api_key.as_bytes()).unwrap());
        let client = Client::builder()
            .default_headers(headers)
            .danger_accept_invalid_certs(true)
            .build().unwrap();
        Self { image_uri, container_uri, client }
    }
}

#[async_trait]
impl Orchestrator for PortainerOrchestrator {

    async fn container(&self, container_id: String) -> Option<OrchestratorContainer> {
        let container_uri = format!("{}/{}/json", self.container_uri, container_id);
        let response = self.client.get(container_uri).send().await;
        let response_data: PortainerGetResponse = response.unwrap().json().await.unwrap();
        Some(OrchestratorContainer {
            id: response_data.id,
            state: response_data.state.status,
            labels: response_data.config.labels,
            image: response_data.image
        })
    }

    async fn containers(&self) -> Option<Vec<OrchestratorContainer>> {
        let list_uri = format!("{}/json?all=true", self.container_uri);
        let response = self.client.get(list_uri).send().await;
        let response_result = match response {
            Ok(data) => data.json().await,
            Err(err) => {
                error!("Portainer error fetching containers: {:?}", err);
                Ok(Vec::new())
            }
        };
        let containers_get = response_result.unwrap_or_default();
        Some(containers_get.into_iter().filter(|c: &OrchestratorContainer| c.is_managed()).collect())
    }

    async fn container_start(&self, connector_id: String) -> () {
        let start_container_uri = format!("{}/{}/start", self.container_uri, connector_id);
        self.client.post(start_container_uri).send().await.unwrap();
    }

    async fn container_deploy(&self, connector: &Connector) -> Option<OrchestratorContainer> {
        let container_env_variables = connector.manager_contract_configuration.clone().unwrap()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect::<Vec<String>>();
        // region First operation, pull the image
        let create_image_uri = format!("{}/create?fromImage={}", self.image_uri, "opencti/connector-ipinfo:latest");
        let mut create_response = self.client.post(create_image_uri).send().await.unwrap();
        while let Some(_chunk) = create_response.chunk().await.unwrap() {} // Iter chunk to fetch all
        // endregion
        // region Deploy the container after success
        let deploy_container_uri = format!("{}/create?name={}", self.container_uri, "my-ipinfo-connector");
        let image_labels = HashMap::from([
            ("opencti-connector-id".into(), connector.id.clone().into_inner())
        ]);
        let json_body = PortainerDeployPayload {
            env: container_env_variables,
            image: "opencti/connector-ipinfo:latest".into(),
            labels: image_labels,
            host_config: PortainerDeployHostConfig {
                // RestartPolicy - will be managed by the composer
                network_mode: Some("opencti-dev_default".into())
            },
        };
        let deploy_response = self.client.post(deploy_container_uri)
            .json(&json_body).send().await.unwrap();
        let deploy_data: PortainerDeployResponse = deploy_response.json().await.unwrap();
        // endregion
        // Return the result
        self.container(deploy_data.id.clone()).await
    }
}