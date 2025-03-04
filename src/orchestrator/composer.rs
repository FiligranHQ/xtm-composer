use crate::api::opencti::connector::{ConnectorCurrentStatus, ConnectorRequestStatus};
use crate::api::{ApiConnector, ComposerApi};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::info;

async fn orchestrate_missing(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
) {
    // Connector is not provisioned, deploy the images
    let connector_id = connector.id.clone();
    info!(id = connector_id, "Deploying the container");
    let deploy_action = orchestrator.deploy(connector).await;
    match deploy_action {
        Some(_) => {
            // Update the connector status
            api.patch_status(connector_id, ConnectorCurrentStatus::Created)
                .await;
        }
        None => {
            info!(id = connector_id, "Deployment canceled")
        }
    }
}

async fn orchestrate_existing(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
    container: OrchestratorContainer,
) {
    // Connector is provisioned
    let connector_id = connector.id.clone();
    let current_status_fetch = connector.current_status.clone().unwrap_or("created".into()); // Default current to created
    let current_connector_status =
        ConnectorCurrentStatus::from_str(current_status_fetch.as_str()).unwrap();
    let requested_status_fetch = connector.requested_status.clone();
    let requested_connector_status =
        ConnectorRequestStatus::from_str(requested_status_fetch.as_str()).unwrap();
    let current_container_status = orchestrator.state_converter(&container);
    // Update the connector status if needed
    let connector_status_is_created = current_connector_status == ConnectorCurrentStatus::Created;
    let container_status_is_stopped = current_container_status != ConnectorCurrentStatus::Started;
    let container_status_is_logic_same = connector_status_is_created && container_status_is_stopped;
    let container_status_not_aligned = current_container_status != current_connector_status;
    if !container_status_is_logic_same && container_status_not_aligned {
        api.patch_status(connector.id.clone(), current_container_status)
            .await;
        info!(id = connector_id, "Patch status");
    }
    // In case of platform upgrade, we need to align all deployed connectors
    let requested_connector_hash = connector.contract_hash.clone();
    let current_container_hash = container.extract_opencti_hash();
    if !requested_connector_hash.eq(current_container_hash) {
        // Versions are not aligned
        info!(
            id = connector_id,
            hash = requested_connector_hash,
            "Refreshing"
        );
        orchestrator.refresh(connector).await;
    }
    // Align existing and requested status
    match (requested_connector_status, current_container_status) {
        (ConnectorRequestStatus::Stopping, ConnectorCurrentStatus::Started) => {
            info!(id = connector_id, "Stopping");
            orchestrator.stop(&container, connector).await;
        }
        (ConnectorRequestStatus::Starting, ConnectorCurrentStatus::Stopped) => {
            info!(id = connector_id, "Starting");
            orchestrator.start(&container, connector).await;
        }
        (ConnectorRequestStatus::Starting, ConnectorCurrentStatus::Created) => {
            info!(id = connector_id, "Starting");
            orchestrator.start(&container, connector).await;
        }
        _ => {
            info!(id = connector_id, "Nothing to execute");
        }
    }
    // Get latest logs and update opencti
    let connector_logs = orchestrator.logs(&container, connector).await;
    match connector_logs {
        Some(logs) => {
            api.patch_logs(connector_id, logs).await;
        }
        None => {
            // No logs
        }
    }
}

pub async fn orchestrate(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
) {
    // Get the current definition from OpenCTI
    let connectors_response = api.connectors().await;
    // First round trip to instantiate and control if needed
    if connectors_response.is_some() {
        let connectors = connectors_response.unwrap();
        // Iter on each definition and check alignment between the status and the container
        for connector in &connectors {
            // Get current containers in the orchestrator
            let container_get = orchestrator.get(connector).await;
            match container_get {
                Some(container) => {
                    orchestrate_existing(orchestrator, api, connector, container).await
                }
                None => orchestrate_missing(orchestrator, api, connector).await,
            }
        }
        // Iter on each existing container to clean the containers
        let connectors_by_id: HashMap<String, ApiConnector> = connectors
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        let existing_containers = orchestrator.list().await.unwrap();
        for container in existing_containers {
            let connector_id = container.extract_opencti_id();
            if !connectors_by_id.contains_key(&connector_id) {
                orchestrator.remove(&container).await;
            }
        }
    } else {
        // Iter on each existing container to clean the containers
        let existing_containers = orchestrator.list().await.unwrap();
        for container in existing_containers {
            orchestrator.remove(&container).await;
        }
    }
}
