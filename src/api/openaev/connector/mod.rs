use rsa::{RsaPrivateKey};
use serde::Deserialize;
use crate::api::ApiConnector;
use crate::api::decrypt_value::resolve_contract_configuration;

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
    pub connector_instance_id: String,
    pub connector_instance_name: String,
    pub connector_instance_hash: String,
    pub connector_image: String,
    pub connector_instance_current_status: String,
    pub connector_instance_requested_status: String,
    pub connector_instance_configurations: Vec<ConnectorContractConfiguration>,
}

impl ConnectorInstances {

    pub fn to_api_connector(&self, private_key: &RsaPrivateKey )->ApiConnector {
        let (contract_configuration, undecryptable_keys) = resolve_contract_configuration(
            private_key,
            &self.connector_instance_id,
            self.connector_instance_configurations.iter().map(|c| {
                (
                    c.configuration_key.clone(),
                    c.configuration_value.clone(),
                    c.configuration_is_encrypted,
                )
            }),
        );
        ApiConnector {
            id: self.connector_instance_id.clone(),
            platform: "openaev".to_string(),
            name: self.connector_instance_name.clone(),
            image: self.connector_image.clone(),
            contract_hash: self.connector_instance_hash.clone(),
            current_status: Some(self.connector_instance_current_status.clone()),
            requested_status: self.connector_instance_requested_status.clone(),
            contract_configuration,
            undecryptable_keys,
        }
    }
}