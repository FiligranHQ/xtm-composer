use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_empty_response;
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
    let add_logs_response = api.post(&format!("/xtm-composer/{}/connector-instances/{}/logs",settings.manager.id, id))
        .json(&logs_input)
        .send()
        .await;

    // The OpenAEV logs endpoint returns an empty body on success:
    // only check the HTTP status instead of parsing JSON.
    let _ = handle_api_empty_response(
        add_logs_response,
        "push logs for connector instance"
    ).await;

    Some(id)
}