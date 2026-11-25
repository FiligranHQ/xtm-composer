use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::connector::ConnectorInstances;

#[derive(Serialize)]
struct ConnectorInstanceHealthInput {
    connector_instance_restart_count: u32,
    connector_instance_started_at: String,
    connector_instance_is_in_reboot_loop: bool,
    /// Digest of the image the workload runs; omitted when unknown, so an OpenAEV
    /// that does not know the field keeps accepting the payload
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_instance_image_digest: Option<String>,
}

pub async fn update_health(
    id: String,
    restart_count: u32,
    started_at: String,
    is_in_reboot_loop: bool,
    image_digest: Option<String>,
    api: &ApiOpenAEV,
)-> Option<String> {
    let settings = crate::settings();
    let health_check_input = ConnectorInstanceHealthInput {
        connector_instance_restart_count: restart_count,
        connector_instance_started_at: started_at,
        connector_instance_is_in_reboot_loop: is_in_reboot_loop,
        connector_instance_image_digest: image_digest,
    };

    let health_check_response = api.put(&format!("/xtm-composer/{}/connector-instances/{}/health-check", settings.manager.id, id))
        .json(&health_check_input)
        .send()
        .await;

    let _ = handle_api_response::<ConnectorInstances>(
        health_check_response,
        "push health metrics"
    ).await;

    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(image_digest: Option<String>) -> serde_json::Value {
        serde_json::to_value(ConnectorInstanceHealthInput {
            connector_instance_restart_count: 1,
            connector_instance_started_at: "2026-11-25T10:00:00Z".to_string(),
            connector_instance_is_in_reboot_loop: false,
            connector_instance_image_digest: image_digest,
        })
        .unwrap()
    }

    #[test]
    fn the_image_digest_is_sent_when_known() {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(
            input(Some(digest.to_string()))["connector_instance_image_digest"],
            serde_json::json!(digest)
        );
    }

    #[test]
    fn an_unknown_image_digest_is_omitted() {
        assert!(input(None).get("connector_instance_image_digest").is_none());
    }
}
