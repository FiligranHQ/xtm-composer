use serde::Serialize;
use crate::api::ApiConnector;
use rsa::{RsaPrivateKey};
use std::str;

pub mod get_listing;
pub mod post_status;
pub mod post_logs;
pub mod post_health;

use cynic;
use crate::api::opencti::opencti as schema;
use crate::api::decrypt_value::resolve_contract_configuration;

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
        let id = self.id.clone().into_inner();
        let (contract_configuration, undecryptable_keys) = resolve_contract_configuration(
            private_key,
            &id,
            self.manager_contract_configuration
                .clone()
                .unwrap()
                .into_iter()
                .map(|c| (c.key, c.value, c.encrypted.unwrap_or_default())),
        );
        ApiConnector {
            id,
            platform: "opencti".to_string(),
            name: self.name.clone(),
            image: self.manager_contract_image.clone().unwrap(),
            contract_hash: self.manager_contract_hash.clone().unwrap(),
            current_status: self.manager_current_status.clone(),
            requested_status: self.manager_requested_status.clone().unwrap(),
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
        let managed = ManagedConnector {
            id: cynic::Id::new("connector-1"),
            name: "Connector".to_string(),
            manager_contract_hash: Some("hash".to_string()),
            manager_contract_image: Some("image".to_string()),
            manager_current_status: Some("started".to_string()),
            manager_requested_status: Some("starting".to_string()),
            manager_contract_configuration: Some(vec![
                ConnectorContractConfiguration {
                    key: "GOOD".to_string(),
                    value: Some(encrypt(1, "s3cr3t")),
                    encrypted: Some(true),
                },
                ConnectorContractConfiguration {
                    key: "BAD".to_string(),
                    value: Some("bm90LWVuY3J5cHRlZA==".to_string()),
                    encrypted: Some(true),
                },
            ]),
        };

        let connector = managed.to_api_connector(test_private_key());

        assert_eq!(connector.undecryptable_keys, vec!["BAD".to_string()]);
        assert_eq!(connector.contract_configuration.len(), 1);
        assert_eq!(connector.contract_configuration[0].value, "s3cr3t");
    }
}
