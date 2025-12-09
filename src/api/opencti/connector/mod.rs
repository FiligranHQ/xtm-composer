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
use crate::api::decrypt_value::parse_aes_encrypted_value;

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
                    let decoded_value_result = parse_aes_encrypted_value(private_key, encrypted_value);
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
