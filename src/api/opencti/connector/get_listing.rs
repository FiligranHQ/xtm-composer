use crate::api::ApiConnector;
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;
use crate::api::opencti::error_handler::{extract_optional_field, handle_graphql_response};
use tracing::error;

// region schema
use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query")]
pub struct GetConnectors {
    pub connectors_for_managers: Option<Vec<ManagedConnector>>,
}
// endregion

pub async fn list(api: &ApiOpenCTI) -> Option<Vec<ApiConnector>> {
    use cynic::QueryBuilder;

    let query = GetConnectors::build({});
    let get_connectors = api.query_fetch(query).await;
    match get_connectors {
        Ok(response) => {
            handle_graphql_response(
                response,
                "connectors_for_managers",
                "OpenCTI backend does not support XTM composer connector listing. The composer cannot manage connectors without backend support."
            ).and_then(|data| {
                extract_optional_field(
                    data.connectors_for_managers,
                    "connectors_for_managers",
                    "connectors_for_managers"
                ).map(|connectors| {
                    connectors
                        .into_iter()
                        .map(|managed_connector| managed_connector.to_api_connector(&api.private_key))
                        .collect()
                })
            })
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to fetch connectors");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::OsRng;
    use rsa::RsaPrivateKey;

    #[test]
    fn hunt_connector_listing_converts_to_a_deployable_connector() {
        let payload = serde_json::json!({
            "connectorsForManagers": [{
                "id": "4f1c2a8e-7d3b-4e5f-9a61-2b8c0d9e7f10",
                "name": "Splunk Hunt",
                "manager_contract_hash": "hash-hunt",
                "manager_contract_image": "opencti/connector-splunk-hunt:rolling",
                "manager_current_status": null,
                "manager_requested_status": "starting",
                "manager_contract_configuration": [
                    { "key": "CONNECTOR_TYPE", "value": "INTERNAL_HUNT", "encrypted": false },
                    { "key": "CONNECTOR_SCOPE", "value": "splunk", "encrypted": false },
                    { "key": "SPLUNK_HUNT_TOKEN", "value": "bm90LWEtY2lwaGVydGV4dA==", "encrypted": true }
                ]
            }]
        });
        let listing: GetConnectors =
            serde_json::from_value(payload).expect("a hunt connector listing should deserialize");
        let private_key = RsaPrivateKey::new(&mut OsRng, 1024).expect("test key");

        let connectors: Vec<ApiConnector> = listing
            .connectors_for_managers
            .expect("connectors should be listed")
            .iter()
            .map(|managed_connector| managed_connector.to_api_connector(&private_key))
            .collect();

        assert_eq!(connectors.len(), 1);
        let connector = &connectors[0];
        assert_eq!(connector.image, "opencti/connector-splunk-hunt:rolling");
        assert_eq!(connector.contract_hash, "hash-hunt");
        assert_eq!(connector.requested_status, "starting");
        let config = |key: &str| {
            connector
                .contract_configuration
                .iter()
                .find(|config| config.key == key)
                .unwrap_or_else(|| panic!("{} should be in the contract", key))
        };
        assert_eq!(config("CONNECTOR_TYPE").value, "INTERNAL_HUNT");
        assert!(!config("CONNECTOR_TYPE").is_sensitive);
        assert_eq!(config("CONNECTOR_SCOPE").value, "splunk");
        // A credential that cannot be decrypted stays sensitive and never exposes its ciphertext.
        assert!(config("SPLUNK_HUNT_TOKEN").is_sensitive);
        assert_eq!(config("SPLUNK_HUNT_TOKEN").value, "");
    }
}
