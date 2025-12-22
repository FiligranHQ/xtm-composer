use crate::api::openaev::api_handler::{handle_api_text_response};
use crate::api::openaev::ApiOpenAEV;

pub async fn get_version(api: &ApiOpenAEV) -> Option<String> {
    let response = api.get("/settings/version").send().await;
    handle_api_text_response(response, "fetch version").await
}