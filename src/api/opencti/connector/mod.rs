use serde::Serialize;
use crate::api::{ApiConnector, ApiContractConfig};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, pkcs1::DecodeRsaPrivateKey};

pub mod get_listing;
pub mod post_status;
pub mod post_logs;

use cynic;
use crate::api::opencti::opencti as schema;

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
    pub fn to_api_connector(&self) -> ApiConnector {
        let settings = crate::settings();
        let priv_key = RsaPrivateKey::from_pkcs1_pem(&settings.manager.credentials_key).unwrap();
        let contract_configuration = self
            .manager_contract_configuration
            .clone()
            .unwrap()
            .into_iter()
            .map(|c|
                if c.encrypted.unwrap_or_default() {
                    let value = c.value.unwrap_or_default();
                    let value_as_bytes = value.as_bytes();
                    let dec_data = priv_key.decrypt(Pkcs1v15Encrypt, &value_as_bytes).expect("failed to decrypt");
                    let dec_data_as_str = str::from_utf8(&dec_data).unwrap().to_string();
                    ApiContractConfig {
                        key: c.key,
                        value: dec_data_as_str,
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