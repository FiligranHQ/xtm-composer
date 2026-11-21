use serde::Serialize;
use tracing::info;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::manager::{ConnectorManager, composer_image, COMPOSER_VERSION};
use crate::api::rsa_key::public_key_spki_pem;

#[derive(Serialize)]
struct RegisterInput {
    id: String,
    name: String,
    public_key: String,
    // Optional for OpenAEV: an OpenAEV that predates them ignores both fields.
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

pub async fn register(api: &ApiOpenAEV) {
    let settings = crate::settings();
    let priv_key = crate::private_key();
    let public_key: String = public_key_spki_pem(priv_key)
        .expect("Failed to encode public key as PKCS#8");

    let register_input = RegisterInput {
        id: settings.manager.id.clone(),
        name: settings.manager.name.clone(),
        public_key,
        version: COMPOSER_VERSION.to_string(),
        image: composer_image(settings.manager.image.as_deref()),
    };

    let register_response = api.post("/xtm-composer/register")
        .json(&register_input)
        .send()
        .await;

    let manager = handle_api_response::<ConnectorManager>(
        register_response,
        "register into OpenAEV backend"
    ).await;

    if let Some(m) = manager {
        info!("Manager registered: {}", m.xtm_composer_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_body_carries_version_and_image() {
        let input = RegisterInput {
            id: "id".into(),
            name: "name".into(),
            public_key: "key".into(),
            version: COMPOSER_VERSION.to_string(),
            image: Some("filigran/xtm-composer:3.261121.0".into()),
        };
        let body = serde_json::to_value(&input).unwrap();
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["image"], "filigran/xtm-composer:3.261121.0");
    }

    #[test]
    fn register_body_omits_an_unknown_image() {
        let input = RegisterInput {
            id: "id".into(),
            name: "name".into(),
            public_key: "key".into(),
            version: COMPOSER_VERSION.to_string(),
            image: None,
        };
        let body = serde_json::to_value(&input).unwrap();
        assert!(body.get("image").is_none());
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    }
}
