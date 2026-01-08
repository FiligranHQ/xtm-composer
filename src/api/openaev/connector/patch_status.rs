use serde::Serialize;
use crate::api::{ApiConnector, ConnectorStatus};
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::connector::ConnectorInstances;
use crate::api::opencti::connector::post_status::ConnectorCurrentStatus;

#[derive(Serialize)]
struct UpdateConnectorInstanceStatusInput {
    connector_instance_current_status: ConnectorCurrentStatus,
}

pub async fn update_status(id: String, status: ConnectorStatus, api: &ApiOpenAEV) -> Option<ApiConnector> {
    let update_status = match status {
        ConnectorStatus::Started => ConnectorCurrentStatus::Started,
        _ => ConnectorCurrentStatus::Stopped,
    };

    let status_input = UpdateConnectorInstanceStatusInput {
        connector_instance_current_status: update_status
    };

    let settings = crate::settings();
    let update_status_response = api.put(&format!("/xtm-composer/{}/connector-instances/{}/status", settings.manager.id, id))
        .json(&status_input)
        .send()
        .await;

    handle_api_response::<ConnectorInstances>(update_status_response, "patch connector instance status")
        .await
        .map(|connector| connector.to_api_connector(&api.private_key))
}