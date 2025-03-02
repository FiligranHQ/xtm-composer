use crate::api::connector;
use crate::api::connector::{update_current_status, ConnectorCurrentStatus, ConnectorRequestStatus, ManagedConnector};
use crate::config::settings::Settings;
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use log::info;
use std::collections::HashMap;
use std::str::FromStr;

async fn orchestrate_missing(settings: &Settings, orchestrator: &Box<dyn Orchestrator>, connector: &ManagedConnector) {
    // Connector is not provisioned, deploy the images
    let connector_id = connector.clone().id.into_inner();
    info!("[X] CONNECTOR IS NOT DEPLOY: {}", connector_id);
    let deploy_action = orchestrator.deploy(settings, connector).await;
    match deploy_action {
        Some(_) => {
            // Update the connector status
            update_current_status(settings, connector_id, ConnectorCurrentStatus::Created).await;
        }
        None => {
            info!("Deployment canceled")
        }
    }
}

async fn orchestrate_existing(settings: &Settings, orchestrator: &Box<dyn Orchestrator>, connector: &ManagedConnector, container: &OrchestratorContainer) {
    // Connector is provisioned
    let cloned_connector = connector.clone();
    let connector_id = cloned_connector.id.into_inner();
    let current_status_fetch = &cloned_connector.manager_current_status.unwrap_or("created".into()); // Default current to created
    let current_connector_status = ConnectorCurrentStatus::from_str(current_status_fetch).unwrap();
    let requested_status_fetch = &cloned_connector.manager_requested_status.unwrap();
    let requested_connector_status = ConnectorRequestStatus::from_str(requested_status_fetch).unwrap();
    let current_container_status = orchestrator.state_converter(container);
    info!("[V] CONNECTOR IS DEPLOY: {} - {} - {}", connector_id, container.image, container.state);
    // Update the connector status if needed
    if current_container_status != current_connector_status {
        update_current_status(settings, connector.id.clone().into_inner(), current_container_status).await;
        info!("[V] CONNECTOR STATUS UPDATED: {} - {:?}", connector.id.inner(), current_container_status);
    }
    // In case of platform upgrade, we need to align all deployed connectors
    let requested_image = &cloned_connector.manager_contract_image.unwrap();
    if !container.image.eq(requested_image) {
        // Versions are not aligned
        info!("[V] CONNECTOR VERSION NOT ALIGNED: {} - Requested: {} - Current: {}", connector.id.inner(), container.image, requested_image);
        // TODO Ask for redeploy
    }
    // Align existing and requested status
    match (requested_connector_status, current_container_status) {
        (ConnectorRequestStatus::Stopping, ConnectorCurrentStatus::Started) => {
            info!("[V] CONNECTOR STARTED {} - Stopping the container", container.id);
            orchestrator.stop(container, connector).await;
        }
        (ConnectorRequestStatus::Starting, ConnectorCurrentStatus::Stopped) => {
            info!("[V] CONNECTOR STOPPED {} - Starting the container", container.id);
            orchestrator.start(container, connector).await;
        }
        (ConnectorRequestStatus::Starting, ConnectorCurrentStatus::Created) => {
            info!("[V] CONNECTOR CREATED {} - Starting the container", container.id);
            orchestrator.start(container, connector).await;
        }
        _ => {
            info!("[V] CONNECTOR {} - Nothing to execute", container.id);
        }
    }
    // Get latest logs and update opencti
    let connector_logs = orchestrator.logs(container, connector).await;
    match connector_logs {
        Some(logs) => {
            connector::update_connector_logs(settings, connector_id, logs).await;
        }
        None => {
            // No logs
        }
    }
}

pub async fn orchestrate(setting: &Settings, orchestrator: &Box<dyn Orchestrator>) {
    // Get the current definition from OpenCTI
    let connectors_response = connector::list(&setting).await.data;
    if connectors_response.is_some() {
        let connectors = connectors_response.unwrap().connectors_for_manager.unwrap_or_default();
        // Iter on each definition and check alignment between the status and the container
        for connector in connectors {
            // Get current containers in the orchestrator
            let containers = orchestrator.list(&connector).await.unwrap_or_default();
            let containers_by_id: HashMap<String, OrchestratorContainer> = containers.into_iter()
                .map(|n| (n.extract_opencti_id().clone(), n.clone())).collect();
            match containers_by_id.get(connector.id.inner()) {
                Some(container) => orchestrate_existing(setting, orchestrator, &connector, container).await,
                None => orchestrate_missing(setting, orchestrator, &connector).await
            }
        }
    }
}