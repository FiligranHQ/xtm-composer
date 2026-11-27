use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::connector::ConnectorInstances;
use crate::orchestrator::ContainerTermination;

#[derive(Serialize)]
struct ConnectorInstanceHealthInput {
    connector_instance_restart_count: u32,
    connector_instance_started_at: String,
    connector_instance_is_in_reboot_loop: bool,
    /// Digest of the image the workload runs; omitted when unknown, so an OpenAEV
    /// that does not know the field keeps accepting the payload
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_instance_image_digest: Option<String>,
    /// How the previous container run ended (`OOMKilled`, `Error`, ...); the three
    /// termination fields are omitted when unknown, so an OpenAEV that does not
    /// know them keeps accepting the payload
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_instance_last_termination_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_instance_last_exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_instance_last_finished_at: Option<String>,
}

impl ConnectorInstanceHealthInput {
    fn new(
        restart_count: u32,
        started_at: String,
        is_in_reboot_loop: bool,
        image_digest: Option<String>,
        last_termination: Option<ContainerTermination>,
    ) -> Self {
        let termination = last_termination.unwrap_or_default();
        Self {
            connector_instance_restart_count: restart_count,
            connector_instance_started_at: started_at,
            connector_instance_is_in_reboot_loop: is_in_reboot_loop,
            connector_instance_image_digest: image_digest,
            connector_instance_last_termination_reason: termination.reason,
            connector_instance_last_exit_code: termination.exit_code,
            connector_instance_last_finished_at: termination.finished_at,
        }
    }
}

pub async fn update_health(
    id: String,
    restart_count: u32,
    started_at: String,
    is_in_reboot_loop: bool,
    image_digest: Option<String>,
    last_termination: Option<ContainerTermination>,
    api: &ApiOpenAEV,
)-> Option<String> {
    let settings = crate::settings();
    let health_check_input = ConnectorInstanceHealthInput::new(
        restart_count,
        started_at,
        is_in_reboot_loop,
        image_digest,
        last_termination,
    );

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

    fn input(
        image_digest: Option<String>,
        last_termination: Option<ContainerTermination>,
    ) -> serde_json::Value {
        serde_json::to_value(ConnectorInstanceHealthInput::new(
            1,
            "2026-11-25T10:00:00Z".to_string(),
            false,
            image_digest,
            last_termination,
        ))
        .unwrap()
    }

    #[test]
    fn the_image_digest_is_sent_when_known() {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(
            input(Some(digest.to_string()), None)["connector_instance_image_digest"],
            serde_json::json!(digest)
        );
    }

    #[test]
    fn an_unknown_image_digest_is_omitted() {
        assert!(input(None, None).get("connector_instance_image_digest").is_none());
    }

    #[test]
    fn the_last_termination_is_sent_when_known() {
        let payload = input(
            None,
            Some(ContainerTermination {
                reason: Some("OOMKilled".to_string()),
                exit_code: Some(137),
                finished_at: Some("2026-11-25T09:59:00Z".to_string()),
            }),
        );
        assert_eq!(payload["connector_instance_last_termination_reason"], "OOMKilled");
        assert_eq!(payload["connector_instance_last_exit_code"], 137);
        assert_eq!(payload["connector_instance_last_finished_at"], "2026-11-25T09:59:00Z");
    }

    #[test]
    fn an_unknown_last_termination_is_omitted() {
        let payload = input(None, None);
        for field in [
            "connector_instance_last_termination_reason",
            "connector_instance_last_exit_code",
            "connector_instance_last_finished_at",
        ] {
            assert!(payload.get(field).is_none(), "{field} should be omitted");
        }
    }
}
