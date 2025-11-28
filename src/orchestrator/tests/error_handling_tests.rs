use rstest::rstest;
use crate::api::{ConnectorStatus, RequestedStatus};

// ============================================================================
// Container Management Tests (Parameterized)
// ============================================================================

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

#[rstest]
#[case("connector-123", "connector-123")]
#[case("test-1", "test-1")]
#[case("abc-def-456", "abc-def-456")]
fn test_orchestrator_container_extract_opencti_id(
    #[case] connector_id: &str,
    #[case] expected_id: &str,
) {
    use super::test_helpers::create_test_container;

    let container = create_test_container(connector_id, "test-container", "running");
    assert_eq!(container.extract_opencti_id(), expected_id);
}

#[test]
fn test_orchestrator_container_extract_opencti_hash() {
    use super::test_helpers::create_test_container;

    let container = create_test_container("test-1", "test-container", "running");
    assert_eq!(container.extract_opencti_hash(), "test-hash-abc123");
}

// ============================================================================
// Reboot Loop Detection Tests (Parameterized)
// ============================================================================

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

#[rstest]
#[case(0, false)]
#[case(1, false)]
#[case(2, false)]
#[case(3, false)]
#[case(4, true)]
#[case(5, true)]
#[case(10, true)]
fn test_orchestrator_container_is_in_reboot_loop_restart_count(
    #[case] restart_count: u32,
    #[case] expected_in_loop: bool,
) {
    use super::test_helpers::create_test_container_with_restarts;
    use chrono::Utc;

    let container = create_test_container_with_restarts(
        "test-1",
        "test-container",
        restart_count,
        Some(Utc::now().to_rfc3339()),
    );

    assert_eq!(container.is_in_reboot_loop(), expected_in_loop);
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

// ============================================================================
// API Connector Tests (Parameterized)
// ============================================================================

#[rstest]
#[case("Test Connector", "test-connector")]
#[case("Test Connector_With Special@Chars", "test-connector-with-special-chars")]
#[case("UPPERCASE_CONNECTOR", "uppercase-connector")]
#[case("multiple   spaces", "multiple---spaces")] // Multiple spaces become multiple hyphens
#[case("dots.and.dashes-test", "dots-and-dashes-test")]
fn test_api_connector_container_name_sanitization(
    #[case] input_name: &str,
    #[case] expected_name: &str,
) {
    use crate::api::{ApiConnector, ApiContractConfig, EnvValue};
    use crate::config::SecretString;

    let connector = ApiConnector {
        id: "test-1".to_string(),
        name: input_name.to_string(),
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
    assert_eq!(container_name, expected_name);
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

// ============================================================================
// Connector Status Tests (Parameterized)
// ============================================================================

#[rstest]
#[case("started", ConnectorStatus::Started)]
#[case("running", ConnectorStatus::Started)]
#[case("healthy", ConnectorStatus::Started)]
#[case("stopped", ConnectorStatus::Stopped)]
#[case("created", ConnectorStatus::Stopped)]
#[case("exited", ConnectorStatus::Stopped)]
#[case("unknown", ConnectorStatus::Stopped)]
fn test_connector_status_from_str(
    #[case] input: &str,
    #[case] expected: ConnectorStatus,
) {
    use crate::api::ConnectorStatus;
    use std::str::FromStr;

    let status = ConnectorStatus::from_str(input).unwrap();
    assert_eq!(status, expected);
}

// ============================================================================
// Requested Status Tests (Parameterized)
// ============================================================================

#[rstest]
#[case("starting", RequestedStatus::Starting)]
#[case("stopping", RequestedStatus::Stopping)]
#[case("unknown", RequestedStatus::Stopping)]
fn test_requested_status_from_str(
    #[case] input: &str,
    #[case] expected: RequestedStatus,
) {
    use crate::api::RequestedStatus;
    use std::str::FromStr;

    let status = RequestedStatus::from_str(input).unwrap();
    assert_eq!(status, expected);
}
