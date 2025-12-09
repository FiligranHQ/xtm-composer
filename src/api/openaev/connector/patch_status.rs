use serde::Serialize;
use tracing::{error, info};
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
    let update_status_response = api.put(&format!("/xtm-composer/{}/connector-instances/{}/status", settings.clone().manager.id, id))
        .json(&status_input)
        .send()
        .await;

    handle_api_response::<ConnectorInstances>(update_status_response, "patch connector instance status")
        .await
        .map(|connector| connector.to_api_connector(&api.private_key))
}

// match update_status_response {
//     Ok(response) => {
//         if response.status().is_success() {
//             match response.json::<ConnectorInstances>().await {
//                 Ok(connector) => {
//                     let instance = connector.to_api_connector(&api.private_key);
//                     info!("Connector instance updated successfully: {:?}", instance);
//                     Some(instance)
//                 }
//                 Err(err) => {
//                     error!(
//                         error = err.to_string(),
//                         "Failed to parse connector instance response"
//                     );
//                     None
//                 }
//             }
//         } else {
//             error!(
//                 status = response.status().as_u16(),
//                 "Failed to fetch patch status"
//             );
//             None
//         }
//     }
//     Err(err) => {
//         error!(
//             error = err.to_string(),
//             "Fail to patch status"
//         );
//         None
//     }
// }