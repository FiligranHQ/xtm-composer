use rsa::{RsaPrivateKey};
use serde::Deserialize;
use tracing::warn;
use crate::api::{ApiConnector, ApiContractConfig};
use crate::api::decrypt_value::parse_aes_encrypted_value;

pub mod get_connector_instances;
pub mod patch_health;
pub mod patch_status;
pub mod post_logs;

#[derive(Debug, Deserialize)]
pub struct ConnectorContractConfiguration {
    pub configuration_key: String,
    pub configuration_value: Option<String>,
    pub configuration_is_encrypted: bool,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorInstances {
    pub connector_instance_id: cynic::Id,
    pub connector_instance_name: String,
    pub connector_instance_hash: Option<String>,
    pub connector_image: Option<String>,
    pub connector_instance_current_status: Option<String>,
    pub connector_instance_requested_status: Option<String>,
    pub connector_instance_configuration: Option<Vec<ConnectorContractConfiguration>>,
}

impl ConnectorInstances {

    pub fn to_api_connector(&self, private_key: &RsaPrivateKey )->ApiConnector {
        let contract_configuration = self
            .connector_instance_configuration
            .as_ref()
            .unwrap()
            .into_iter()
            .map(|c| {
                let is_sensitive = c.configuration_is_encrypted.clone();
                if is_sensitive {
                    let encrypted_value = c.configuration_value.clone().unwrap_or_default();
                    let decoded_value_result = parse_aes_encrypted_value(private_key, encrypted_value);
                    println!("Configuration key: {}", c.configuration_key.clone());
                    println!("Decoded value result: {:?}", decoded_value_result);
                    match decoded_value_result {
                        Ok(decoded_value) => ApiContractConfig {
                            key: c.configuration_key.clone(),
                            value: decoded_value,
                            is_sensitive: true,
                        },
                        Err(e) => {
                            warn!(error = e.to_string(), "Fail to decode value");
                            ApiContractConfig {
                                key: c.configuration_key.clone(),
                                value: String::from(""),
                                is_sensitive: true,
                            }
                        }
                    }
                } else {
                    ApiContractConfig {
                        key: c.configuration_key.clone(),
                        value: c.configuration_value.clone().unwrap_or_default(),
                        is_sensitive: false,
                    }
                }
            })
            .collect();
        ApiConnector {
            id: self.connector_instance_id.clone().into_inner(),
            name: self.connector_instance_name.clone(),
            image: self.connector_image.clone().unwrap(),
            contract_hash: self.connector_instance_hash.clone().unwrap(),
            current_status: self.connector_instance_current_status.clone(),
            requested_status: self.connector_instance_requested_status.clone().unwrap(),
            contract_configuration,
        }
    }
}