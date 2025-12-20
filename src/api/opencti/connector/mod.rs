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

    pub fn parse_aes_encrypted_value(&self,  private_key: &RsaPrivateKey, encrypted_value: String) -> Result<String, Box<dyn std::error::Error>> {
        let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_value)?;

        // Minimum expected length: 1 (version) + 512 (RSA encrypted key/IV) + 1 (at least some encrypted data)
        // Warn and not panic when length is encrypted data too short for expected format
        if encrypted_bytes.len() < 513 {
            warn!(
                actual_length = encrypted_bytes.len(),
                expected_min_length = 513,
                "Encrypted data too short for expected format"
            );
            return Ok(String::from(""));
        }

        let version = encrypted_bytes[0];
        if version != 1 {
            warn!(version, "Encryption version not handled");
            Ok(String::from(""))
        } else {
            let aes_key_iv_encrypted_bytes = &encrypted_bytes[1..=512];
            let aes_key_iv_decrypted_bytes = private_key.decrypt(Pkcs1v15Encrypt, &aes_key_iv_encrypted_bytes)?;
            let aes_key_bytes = &aes_key_iv_decrypted_bytes[0..32];
            let aes_iv_bytes = &aes_key_iv_decrypted_bytes[32..44];
            let encrypted_value_bytes = &encrypted_bytes[513..];

            let cipher = Aes256Gcm::new_from_slice(&aes_key_bytes)?;
            let nonce = Nonce::from_slice(&aes_iv_bytes);
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
                    let decoded_value_result = self.parse_aes_encrypted_value(private_key, encrypted_value);
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
