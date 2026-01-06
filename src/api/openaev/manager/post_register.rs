use rsa::{RsaPublicKey};
use rsa::pkcs1::LineEnding;
use rsa::pkcs8::EncodePublicKey;
use serde::Serialize;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::ApiOpenAEV;
use crate::api::openaev::manager::ConnectorManager;

#[derive(Serialize)]
struct RegisterInput {
    id: String,
    name: String,
    public_key: String,
}

pub async fn register(api: &ApiOpenAEV) {
    let settings = crate::settings();
    let priv_key = crate::private_key();
    let pub_key = RsaPublicKey::from(priv_key);
    let public_key: String = pub_key
        .to_public_key_pem(LineEnding::LF)
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

    // Discard the result
    let _ = handle_api_response::<ConnectorManager>(
        register_response,
        "register into OpenAEV backend"
    ).await;
}

