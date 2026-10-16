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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::decrypt_value::tests::{encrypt, test_private_key};

    #[test]
    fn to_api_connector_reports_undecryptable_keys() {
        let instance = ConnectorInstances {
            connector_instance_id: "instance-1".to_string(),
            connector_instance_name: "Instance".to_string(),
            connector_instance_hash: "hash".to_string(),
            connector_image: "image".to_string(),
            connector_instance_current_status: "started".to_string(),
            connector_instance_requested_status: "starting".to_string(),
            connector_instance_configurations: vec![
                ConnectorContractConfiguration {
                    configuration_key: "GOOD".to_string(),
                    configuration_value: Some(encrypt(2, "s3cr3t")),
                    configuration_is_encrypted: true,
                },
                ConnectorContractConfiguration {
                    configuration_key: "BAD".to_string(),
                    configuration_value: Some("bm90LWVuY3J5cHRlZA==".to_string()),
                    configuration_is_encrypted: true,
                },
            ],
        };

        let connector = instance.to_api_connector(test_private_key());

        assert_eq!(connector.undecryptable_keys, vec!["BAD".to_string()]);
        assert_eq!(connector.contract_configuration.len(), 1);
        assert_eq!(connector.contract_configuration[0].value, "s3cr3t");
    }
}
