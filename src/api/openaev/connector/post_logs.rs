use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::JSON;
use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;

#[derive(Serialize)]
struct InstanceConnectorLogsInput {
    connector_instance_logs: Vec<String>,
}

pub async fn add_logs(id: String, logs: Vec<String>, api: &ApiOpenAEV)-> Option<String> {
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

    Some(id)
}