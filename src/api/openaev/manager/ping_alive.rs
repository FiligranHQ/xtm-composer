use serde::Serialize;
use crate::api::openaev::api_handler::{handle_api_response};
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::manager::{ConnectorManager, composer_image, COMPOSER_VERSION};

/// What the composer reports about itself on each connectivity refresh, so OpenAEV keeps the
/// running version and image current between registrations. An OpenAEV that predates it reads
/// no body and ignores it.
#[derive(Serialize)]
struct ConnectivityInput {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

pub async fn ping_alive(api: &ApiOpenAEV) -> Option<String> {
    let settings = crate::settings();
    let input = ConnectivityInput {
        version: COMPOSER_VERSION.to_string(),
        image: composer_image(settings.manager.image.as_deref()),
    };
    let response = api.put(&format!("/xtm-composer/{}/refresh-connectivity", settings.manager.id))
        .json(&input)
        .send()
        .await;

    // xtm_composer_version is the platform version the composer registered against: the alive
    // loop registers again when it changes.
    handle_api_response::<ConnectorManager>(response, "ping OpenAEV backend")
        .await
        .map(|manager| manager.xtm_composer_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connectivity_body_carries_version_and_image() {
        let input = ConnectivityInput {
            version: COMPOSER_VERSION.to_string(),
            image: composer_image(Some(" filigran/xtm-composer:rolling ")),
        };
        let body = serde_json::to_value(&input).unwrap();
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["image"], "filigran/xtm-composer:rolling");
    }
}
