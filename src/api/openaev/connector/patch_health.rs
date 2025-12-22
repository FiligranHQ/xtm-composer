use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::connector::ConnectorInstances;

#[derive(Serialize)]
struct ConnectorInstanceHealthInput {
    connector_instance_restart_count: u32,
    connector_instance_started_at: String,
    connector_instance_is_in_reboot_loop: bool
}

pub async fn update_health(
    id: String,
    restart_count: u32,
    started_at: String,
    is_in_reboot_loop: bool,
    api: &ApiOpenAEV,
)-> Option<cynic::Id> {
    let settings = crate::settings();
    let health_check_input = ConnectorInstanceHealthInput {
        connector_instance_restart_count: restart_count,
        connector_instance_started_at: started_at,
        connector_instance_is_in_reboot_loop: is_in_reboot_loop
    };

    let health_check_response = api.put(&format!("/xtm-composer/{}/connector-instances/{}/health-check", settings.clone().manager.id, id.clone()))
        .json(&health_check_input)
        .send()
        .await;

    let _ = handle_api_response::<ConnectorInstances>(
        health_check_response,
        "push health metrics"
    ).await;

    Some(cynic::Id::new(id))
}