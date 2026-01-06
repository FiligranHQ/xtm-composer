use crate::api::openaev::api_handler::{handle_api_text_response};
use crate::api::openaev::ApiOpenAEV;

pub async fn notify_container_removed(id: String, api: &ApiOpenAEV) {
    let settings = crate::settings();
    let response = api.delete(&format!("/xtm-composer/{}/connector-instances/{}", settings.manager.id, id))
        .send()
        .await;

    let _ = handle_api_text_response(
        response,
        "Notify OpenAEV that the container has been successfully removed"
    ).await;
}