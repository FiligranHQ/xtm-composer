use crate::api::openaev::api_handler::{handle_api_response};
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::manager::ConnectorManager;

pub async fn ping_alive(api: &ApiOpenAEV) -> Option<String> {
    let settings = crate::settings();
    let response = api.put(&format!("/xtm-composer/{}/refresh-connectivity", settings.manager.id))
        .send()
        .await;

    handle_api_response::<ConnectorManager>(response, "ping OpenAEV backend")
        .await
        .map(|manager| manager.xtm_composer_version)
}