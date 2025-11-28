#[cfg(test)]
mod registry_resolver;

#[cfg(test)]
mod composer_tests;

#[cfg(test)]
mod error_handling_tests;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod kubernetes_secrets;

#[cfg(test)]
pub mod test_helpers {
    use crate::api::ApiConnector;
    use crate::orchestrator::OrchestratorContainer;
    use std::collections::HashMap;

    pub fn create_test_connector(id: &str, image: &str) -> ApiConnector {
        use crate::api::{ApiContractConfig, EnvValue};
        use crate::config::SecretString;
        
        ApiConnector {
            id: id.to_string(),
            name: format!("test-connector-{}", id),
            image: image.to_string(),
            requested_status: "starting".to_string(),
            current_status: Some("stopped".to_string()),
            contract_hash: "test-hash-abc123".to_string(),
            contract_configuration: vec![
                ApiContractConfig {
                    key: "OPENCTI_TOKEN".to_string(),
                    value: EnvValue::Secret(SecretString::new("test-token".to_string())),
                },
            ],
        }
    }

    pub fn create_test_container(id: &str, name: &str, state: &str) -> OrchestratorContainer {
        let mut labels = HashMap::new();
        labels.insert("opencti-connector-id".to_string(), id.to_string());
        labels.insert("opencti-manager".to_string(), "test-manager".to_string());

        let mut envs = HashMap::new();
        envs.insert(
            "OPENCTI_CONFIG_HASH".to_string(),
            "test-hash-abc123".to_string(),
        );

        OrchestratorContainer {
            id: format!("container-{}", id),
            name: name.to_string(),
            state: state.to_string(),
            labels,
            envs,
            restart_count: 0,
            started_at: Some("2024-01-01T00:00:00Z".to_string()),
        }
    }

    pub fn create_test_container_with_restarts(
        id: &str,
        name: &str,
        restart_count: u32,
        started_at: Option<String>,
    ) -> OrchestratorContainer {
        let mut container = create_test_container(id, name, "running");
        container.restart_count = restart_count;
        container.started_at = started_at;
        container
    }
}
