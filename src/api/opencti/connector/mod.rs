use serde::Serialize;
use crate::api::{ApiConnector, ApiContractConfig};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use tracing::{warn};

pub mod get_listing;
pub mod post_status;
pub mod post_logs;

use cynic;
use base64::{Engine as _,engine::{self, general_purpose}};
use crate::api::opencti::{opencti as schema, ApiOpenCTI};

#[derive(cynic::QueryFragment, Debug, Clone, Serialize)]
pub struct ConnectorContractConfiguration {
    pub key: String,
    pub value: Option<String>,
    pub encrypted: Option<bool>
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

    pub fn parse_encrypted_field(&self,  private_key: &RsaPrivateKey, encrypted_value: String) -> String {
        let encrypted_bytes_result = general_purpose::STANDARD.decode(encrypted_value);
        match encrypted_bytes_result {
            Ok(encrypted_bytes) => {
                let decoded_data_result = private_key.decrypt(Pkcs1v15Encrypt, &encrypted_bytes);
                match decoded_data_result {
                    Ok(decoded_data) => {
                        let dec_data_as_str = str::from_utf8(&decoded_data).unwrap().to_string();
                        dec_data_as_str
                    }
                    Err(..) => {
                        warn!("Incorrect encrypted data decrypt");
                        String::from("")
                    }
                }
            }
            Err(..) => {
                warn!("Incorrect value bas64 decode");
                String::from("")
            }
        }
    }

    pub fn to_api_connector(&self, private_key: &RsaPrivateKey) -> ApiConnector {
        let contract_configuration = self
            .manager_contract_configuration
            .clone()
            .unwrap()
            .into_iter()
            .map(|c|
                if c.encrypted.unwrap_or_default() {
                    let value = c.value.unwrap_or_default();
                    let decoded_value = self.parse_encrypted_field(private_key, value);
                    ApiContractConfig {
                        key: c.key,
                        value: decoded_value,
                    }
                } else {
                    ApiContractConfig {
                        key: c.key,
                        value: c.value.unwrap_or_default(),
                    }
                }
            )
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