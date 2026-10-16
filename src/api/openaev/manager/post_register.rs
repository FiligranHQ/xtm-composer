use serde::Serialize;
use tracing::info;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::manager::ConnectorManager;
use crate::api::rsa_key::public_key_spki_pem;

#[derive(Serialize)]
struct RegisterInput {
    id: String,
    name: String,
    public_key: String,
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

