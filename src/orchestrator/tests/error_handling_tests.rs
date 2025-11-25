// Error handling tests - these will be completed after TASK-6 error handling refactoring

#[test]
fn test_orchestrator_container_is_managed() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-1", "test-container", "running");
    assert!(container.is_managed());

    // Container without opencti-connector-id label should not be managed
    let mut unmanaged = container.clone();
    unmanaged.labels.remove("opencti-connector-id");
    assert!(!unmanaged.is_managed());
}

#[test]
fn test_orchestrator_container_extract_opencti_id() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("connector-123", "test-container", "running");
    assert_eq!(container.extract_opencti_id(), "connector-123");
}

#[test]
fn test_orchestrator_container_extract_opencti_hash() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-1", "test-container", "running");
    assert_eq!(container.extract_opencti_hash(), "test-hash-abc123");
}

#[test]
fn test_orchestrator_container_is_in_reboot_loop_high_restarts() {
    use super::test_helpers::create_test_container_with_restarts;
    use chrono::Utc;

    // High restart count with recent start time = reboot loop
    let container = create_test_container_with_restarts(
        "test-1",
        "test-container",
        5,
        Some(Utc::now().to_rfc3339()),
    );

    assert!(container.is_in_reboot_loop());
}

#[test]
fn test_orchestrator_container_is_in_reboot_loop_old_start() {
    use super::test_helpers::create_test_container_with_restarts;

    // High restart count but old start time = not in reboot loop
    let container = create_test_container_with_restarts(
        "test-1",
        "test-container",
        5,
        Some("2024-01-01T00:00:00Z".to_string()),
    );

    assert!(!container.is_in_reboot_loop());
}

#[test]
fn test_orchestrator_container_is_in_reboot_loop_low_restarts() {
    use super::test_helpers::create_test_container_with_restarts;
    use chrono::Utc;

    // Low restart count even with recent start = not in reboot loop
    let container = create_test_container_with_restarts(
        "test-1",
        "test-container",
        2,
        Some(Utc::now().to_rfc3339()),
    );

    assert!(!container.is_in_reboot_loop());
}

#[test]
fn test_orchestrator_container_is_in_reboot_loop_no_started_at() {
    use super::test_helpers::create_test_container_with_restarts;

    // High restart count but no started_at = not in reboot loop
    let container = create_test_container_with_restarts("test-1", "test-container", 5, None);

    assert!(!container.is_in_reboot_loop());
}

#[test]
fn test_orchestrator_container_is_in_reboot_loop_invalid_timestamp() {
    use super::test_helpers::create_test_container_with_restarts;

    // High restart count with invalid timestamp = not in reboot loop
    let container = create_test_container_with_restarts(
        "test-1",
        "test-container",
        5,
        Some("invalid-timestamp".to_string()),
    );

    assert!(!container.is_in_reboot_loop());
}

#[test]
fn test_api_connector_container_name_sanitization() {
    use crate::api::{ApiConnector, ApiContractConfig, EnvValue};
    use crate::config::SecretString;

    // Create a connector with a name that needs sanitization
    let connector = ApiConnector {
        id: "test-1".to_string(),
        name: "Test Connector_With Special@Chars".to_string(),
        image: "nginx:latest".to_string(),
        requested_status: "starting".to_string(),
        current_status: Some("stopped".to_string()),
        contract_hash: "test-hash-abc123".to_string(),
        contract_configuration: vec![
            ApiContractConfig {
                key: "OPENCTI_TOKEN".to_string(),
                value: EnvValue::Secret(SecretString::new("test-token".to_string())),
            },
        ],
    };
    
    let container_name = connector.container_name();

    // Should be lowercase and alphanumeric with hyphens
    assert!(container_name.chars().all(|c| c.is_lowercase() || c.is_ascii_digit() || c == '-'));
    assert!(!container_name.contains(' '));
    assert!(!container_name.contains('_'));
    assert!(!container_name.contains('@'));
    assert_eq!(container_name, "test-connector-with-special-chars");
}

#[test]
fn test_api_connector_container_envs_includes_hash() {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-1", "nginx:latest");
    let envs = connector.container_envs();

    // Should include OPENCTI_CONFIG_HASH
    let hash_env = envs.iter().find(|e| e.key == "OPENCTI_CONFIG_HASH");
    assert!(hash_env.is_some());
    assert_eq!(hash_env.unwrap().value.as_str(), "test-hash-abc123");
}

#[test]
fn test_api_connector_container_envs_includes_opencti_url() {
    use super::test_helpers::create_test_connector;

    let connector = create_test_connector("test-1", "nginx:latest");
    let envs = connector.container_envs();

    // Should include OPENCTI_URL from settings
    let url_env = envs.iter().find(|e| e.key == "OPENCTI_URL");
    assert!(url_env.is_some());
}

#[test]
fn test_connector_status_from_str() {
    use crate::api::ConnectorStatus;
    use std::str::FromStr;

    assert_eq!(
        ConnectorStatus::from_str("started").unwrap(),
        ConnectorStatus::Started
    );
    assert_eq!(
        ConnectorStatus::from_str("running").unwrap(),
        ConnectorStatus::Started
    );
    assert_eq!(
        ConnectorStatus::from_str("healthy").unwrap(),
        ConnectorStatus::Started
    );
    assert_eq!(
        ConnectorStatus::from_str("stopped").unwrap(),
        ConnectorStatus::Stopped
    );
    assert_eq!(
        ConnectorStatus::from_str("created").unwrap(),
        ConnectorStatus::Stopped
    );
    assert_eq!(
        ConnectorStatus::from_str("exited").unwrap(),
        ConnectorStatus::Stopped
    );
    assert_eq!(
        ConnectorStatus::from_str("unknown").unwrap(),
        ConnectorStatus::Stopped
    );
}

#[test]
fn test_requested_status_from_str() {
    use crate::api::RequestedStatus;
    use std::str::FromStr;

    assert_eq!(
        RequestedStatus::from_str("starting").unwrap(),
        RequestedStatus::Starting
    );
    assert_eq!(
        RequestedStatus::from_str("stopping").unwrap(),
        RequestedStatus::Stopping
    );
    assert_eq!(
        RequestedStatus::from_str("unknown").unwrap(),
        RequestedStatus::Stopping
    );
}

// Additional error handling tests to be added after TASK-6
// These will test error propagation, error types, error messages, etc.
