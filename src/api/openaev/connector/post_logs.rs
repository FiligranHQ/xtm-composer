use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::JSON;
use serde::Serialize;
use serde_json::json;
use tracing::{error, info};
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::connector::ConnectorInstances;

#[derive(Serialize)]
struct InstanceConnectorLogsInput {
    connector_instance_logs: Vec<String>,
}

pub async fn add_logs(id: String, logs: Vec<String>, api: &ApiOpenAEV)-> Option<cynic::Id> {
    let logs_input = InstanceConnectorLogsInput{
        connector_instance_logs: logs
    };
    let settings = crate::settings();
    let add_logs_response = api.post(&format!("/xtm-composer/{}/connector-instances/{}/logs",settings.clone().manager.id, id))
        .json(&logs_input)
        .send()
        .await;

    // Discard the result
    let _ = handle_api_response::<JSON>(
        add_logs_response,
        "push logs for connector instance"
    ).await;

    Some(cynic::Id::new(id))
    // match add_logs_response {
    //     Ok(response) => {
    //         if response.status().is_success() {
    //             info!("Successfully pushed logs for connector instance: {}", id);
    //             Some(cynic::Id::new(id))
    //         } else {
    //             let status = response.status();
    //             let body = response.text().await.unwrap_or_default();
    //             error!(
    //                 status = status.as_u16(),
    //                 body = body,
    //                 "Failed to push logs for connector instance: {}",
    //                 id
    //             );
    //             None
    //         }
    //     }
    //     Err(e) => {
    //         error!(error = e.to_string(), "Failed to send request to push logs");
    //         None
    //     }
    // }
}