use crate::api::{ApiConnector, ComposerApi, ConnectorStatus, RequestedStatus};
use crate::orchestrator::{Orchestrator, OrchestratorContainer};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tracing::{info, warn};

async fn orchestrate_missing(
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
) {
    // Connector is not provisioned, deploy the images
    let id = connector.id.clone();
    info!(id = id, "Deploying the container");
    let deploy_action = orchestrator.deploy(connector).await;
    match deploy_action {
        // Update the connector status
        Some(_) => {
            api.patch_status(id, ConnectorStatus::Stopped).await;
        }
        None => {
            warn!(id = id, "Deployment canceled");
        }
    }
}

async fn orchestrate_existing(
    tick: &mut Instant,
    health_tick: &mut Instant,
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
    connector: &ApiConnector,
    container: OrchestratorContainer,
) {
    // Connector is provisioned
    let connector_id = connector.id.clone();
    let current_status_fetch = connector.current_status.clone().unwrap_or("stopped".into()); // Default current to created
    let connector_status = ConnectorStatus::from_str(current_status_fetch.as_str()).unwrap();
    let requested_status_fetch = connector.requested_status.clone();
    let container_status = orchestrator.state_converter(&container);
    // Check for reboot loop and send health metrics
    let is_in_reboot_loop = container.is_in_reboot_loop();
    let final_status = if is_in_reboot_loop {
        warn!(
            id = connector_id,
            restart_count = container.restart_count,
            "Reboot loop detected"
        );
        // For now, we still report it as Started but with a warning log
        // In the future, we could add a new status like ConnectorStatus::Critical
        container_status
    } else {
        container_status
    };
    
    // Update the connector status if needed
    let container_status_not_aligned = final_status != connector_status;
    
    // Detect if connector just started
    let just_started = container_status_not_aligned && 
                       final_status == ConnectorStatus::Started && 
                       connector_status == ConnectorStatus::Stopped;
    
    // Send health metrics if:
    // - Connector just started (immediate reporting)
    // - OR connector is running and 30 seconds have elapsed
    let now = Instant::now();
    let should_send_health = just_started || 
        (final_status == ConnectorStatus::Started && 
         now.duration_since(health_tick.clone()) >= Duration::from_secs(30));
    
    if should_send_health {
        if let Some(started_at) = &container.started_at {
            info!(id = connector_id, "Reporting health metrics");
            api.patch_health(
                connector_id.clone(),
                container.restart_count,
                started_at.clone(),
                is_in_reboot_loop,
            ).await;
        }
        // Reset timer only for running connectors
        if final_status == ConnectorStatus::Started {
            *health_tick = now;
        }
    }
    if container_status_not_aligned {
        api.patch_status(connector.id.clone(), final_status)
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
    let requested_status = RequestedStatus::from_str(requested_status_fetch.as_str()).unwrap();
    match (requested_status, container_status) {
        (RequestedStatus::Stopping, ConnectorStatus::Started) => {
            info!(id = connector_id, "Stopping");
            orchestrator.stop(&container, connector).await;
        }
        (RequestedStatus::Starting, ConnectorStatus::Stopped) => {
            info!(id = connector_id, "Starting");
            orchestrator.start(&container, connector).await;
        }
        _ => {
            info!(id = connector_id, "Nothing to execute");
        }
    }
    // Get latest logs and update opencti every 5 minutes
    let now = Instant::now();
    if now.duration_since(tick.clone()) >= api.post_logs_schedule() {
        let connector_logs = orchestrator.logs(&container, connector).await;
        match connector_logs {
            Some(logs) => {
                info!(id = connector_id, "Reporting logs");
                api.patch_logs(connector_id, logs).await;
            }
            None => {
                // No logs
            }
        }
        *tick = now;
    }
}

pub async fn orchestrate(
    tick: &mut Instant,
    health_tick: &mut Instant,
    orchestrator: &Box<dyn Orchestrator + Send + Sync>,
    api: &Box<dyn ComposerApi + Send + Sync>,
) {
    // Get the current definition from OpenCTI
    let connectors_response = api.connectors().await;
    if connectors_response.is_some() {
        // First round trip to instantiate and control if needed
        let connectors = connectors_response.unwrap();
        // Iter on each definition and check alignment between the status and the container
        for connector in &connectors {
            // Get current containers in the orchestrator
            let container_get = orchestrator.get(connector).await;
            match container_get {
                Some(container) => {
                    orchestrate_existing(tick, health_tick, orchestrator, api, connector, container).await
                }
                None => orchestrate_missing(orchestrator, api, connector).await,
            }
        }
        // Iter on each existing container to clean the containers
        let connectors_by_id: HashMap<String, ApiConnector> = connectors
            .iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();
        let existing_containers = orchestrator.list().await;
        for container in existing_containers {
            let connector_id = container.extract_opencti_id();
            if !connectors_by_id.contains_key(&connector_id) {
                if orchestrator.remove(&container).await {
                    api.container_removed_successfully(connector_id).await;
                }
            }
        }
    }
}
