use serde::Serialize;
use crate::api::{ApiConnector, ApiContractConfig};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use tracing::{warn};
use std::str;

pub mod get_listing;
pub mod post_status;
pub mod post_logs;
pub mod post_health;

use cynic;
use base64::{Engine as _, engine::general_purpose};
use crate::api::opencti::opencti as schema;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce
};

#[derive(cynic::QueryFragment, Debug, Clone, Serialize)]
pub struct ConnectorContractConfiguration {
    pub key: String,
    pub value: Option<String>,
    pub encrypted: Option<bool>,
    #[cynic(rename = "encryptionKey")]
    pub encryption_key: Option<String>,
    #[cynic(rename = "encryptionIv")]
    pub encryption_iv: Option<String>
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ManagedConnector {
    pub id: cynic::Id,
    pub name: String,
    #[cynic(rename = "manager_contract_hash")]
    pub manager_contract_hash: Option<String>,
    #[cynic(rename = "manager_contract_image")]
    pub manager_contract_image: Option<String>,
    #[cynic(rename = "manager_current_status")]
    pub manager_current_status: Option<String>,
    #[cynic(rename = "manager_requested_status")]
    pub manager_requested_status: Option<String>,
    #[cynic(rename = "manager_contract_configuration")]
    pub manager_contract_configuration: Option<Vec<ConnectorContractConfiguration>>,
}

impl ManagedConnector {

    pub fn parse_aes_encrypted_value(&self,  private_key: &RsaPrivateKey, encrypted_value: String, encrypted_aes_key: String, encrypted_aes_iv: String) -> Result<String, Box<dyn std::error::Error>> {
        let aes_key_encrypted_bytes = general_purpose::STANDARD.decode(encrypted_aes_key)?;
        let aes_key_decrypted_bytes = private_key.decrypt(Pkcs1v15Encrypt, &aes_key_encrypted_bytes)?;

        let aes_iv_encrypted_bytes = general_purpose::STANDARD.decode(encrypted_aes_iv)?;
        let aes_iv_decrypted_bytes = private_key.decrypt(Pkcs1v15Encrypt, &aes_iv_encrypted_bytes)?;

        let encrypted_value_bytes = general_purpose::STANDARD.decode(encrypted_value)?;

        let cipher = Aes256Gcm::new_from_slice(&aes_key_decrypted_bytes)?;
        let nonce = Nonce::from_slice(&aes_iv_decrypted_bytes);
        let plaintext_result = cipher.decrypt(&nonce, encrypted_value_bytes.as_ref());
        match plaintext_result {
            Ok(plaintext) => {
                let decoded_value = str::from_utf8(&plaintext).unwrap().to_string();
                Ok(decoded_value)
            },
            Err(e) => {
                warn!(error = e.to_string(), "Fail to decode value");
                Ok(String::from(""))
            }
        }
    }

    pub fn to_api_connector(&self, private_key: &RsaPrivateKey) -> ApiConnector {
        let contract_configuration = self
            .manager_contract_configuration
            .clone()
            .unwrap()
            .into_iter()
            .map(|c| {
                let is_sensitive = c.encrypted.unwrap_or_default();
                if is_sensitive {
                    let encrypted_value = c.value.unwrap_or_default();
                    let encrypted_key = c.encryption_key.unwrap_or_default();
                    let encrypted_iv = c.encryption_iv.unwrap_or_default();
                    let decoded_value_result = self.parse_aes_encrypted_value(private_key, encrypted_value, encrypted_key, encrypted_iv);
                    match decoded_value_result {
                        Ok(decoded_value) => ApiContractConfig {
                            key: c.key,
                            value: decoded_value,
                            is_sensitive: true,
                        },
                        Err(e) => {
                            warn!(error = e.to_string(), "Fail to decode value");
                            ApiContractConfig {
                                key: c.key,
                                value: String::from(""),
                                is_sensitive: true,
                            }
                        }
                    }
                } else {
                    ApiContractConfig {
                        key: c.key,
                        value: c.value.unwrap_or_default(),
                        is_sensitive: false,
                    }
                }
            })
            .collect();
        ApiConnector {
            id: self.id.clone().into_inner(),
            name: self.name.clone(),
            image: self.manager_contract_image.clone().unwrap(),
            contract_hash: self.manager_contract_hash.clone().unwrap(),
            current_status: self.manager_current_status.clone(),
            requested_status: self.manager_requested_status.clone().unwrap(),
            contract_configuration,
        }
    }
}
