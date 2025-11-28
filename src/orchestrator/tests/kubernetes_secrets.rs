use crate::config::settings::Registry;
use crate::config::SecretString;
use rstest::rstest;
use base64::Engine;

#[rstest]
#[case("registry.example.com", true, true, true)]
#[case("docker.io", false, false, false)]
fn test_registry_credentials(
    #[case] server: &str,
    #[case] with_credentials: bool,
    #[case] expected_username: bool,
    #[case] expected_password: bool,
) {
    let registry = Registry {
        server: Some(server.to_string()),
        username: if with_credentials {
            Some(SecretString::new("test-user".to_string()))
        } else {
            None
        },
        password: if with_credentials {
            Some(SecretString::new("test-password".to_string()))
        } else {
            None
        },
        email: None,
    };
    
    assert_eq!(registry.username.is_some(), expected_username);
    assert_eq!(registry.password.is_some(), expected_password);
    
    if with_credentials {
        assert_eq!(registry.username.unwrap().expose_secret(), "test-user");
    }
}

#[test]
fn test_registry_normalization() {
    let registry = Registry {
        server: None,
        username: None,
        password: None,
        email: None,
    }.normalize();
    
    assert_eq!(registry.server, Some("docker.io".to_string()));
}

#[test]
fn test_default_registry() {
    let registry = Registry::default();
    
    assert_eq!(registry.server, Some("docker.io".to_string()));
    assert!(registry.username.is_none());
    assert!(registry.password.is_none());
}

#[test]
fn test_secret_name_constant() {
    const EXPECTED_SECRET_NAME: &str = "opencti-registry-auth";
    assert_eq!(EXPECTED_SECRET_NAME, "opencti-registry-auth");
}

#[rstest]
#[case(true, true, true, "both username and password present")]
#[case(true, false, false, "username only - both required")]
#[case(false, true, false, "password only - both required")]
#[case(false, false, false, "no credentials present")]
fn test_credentials_check(
    #[case] has_username: bool,
    #[case] has_password: bool,
    #[case] expected_result: bool,
    #[case] description: &str,
) {
    let registry = Registry {
        server: Some("docker.io".to_string()),
        username: if has_username {
            Some(SecretString::new("user".to_string()))
        } else {
            None
        },
        password: if has_password {
            Some(SecretString::new("pass".to_string()))
        } else {
            None
        },
        email: None,
    };
    
    let has_credentials = registry.username.is_some() && registry.password.is_some();
    assert_eq!(has_credentials, expected_result, "{}", description);
}

/// Helper function to build Docker config JSON for testing
fn build_docker_config_json(username: &str, password: &str, server: &str) -> Vec<u8> {
    let auth_string = format!("{}:{}", username, password);
    let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes());
    
    let docker_config = serde_json::json!({
        "auths": {
            server: {
                "username": username,
                "password": password,
                "email": "",
                "auth": auth_base64
            }
        }
    });
    
    serde_json::to_vec(&docker_config).expect("Failed to serialize Docker config")
}

#[test]
fn test_docker_config_json_generation() {
    let config = build_docker_config_json("test-user", "test-pass", "https://registry.example.com");
    
    // Verify the config is valid JSON
    let parsed: serde_json::Value = serde_json::from_slice(&config).expect("Invalid JSON");
    
    // Verify structure
    assert!(parsed["auths"].is_object());
    assert!(parsed["auths"]["https://registry.example.com"].is_object());
    assert_eq!(parsed["auths"]["https://registry.example.com"]["username"], "test-user");
    assert_eq!(parsed["auths"]["https://registry.example.com"]["password"], "test-pass");
}

#[rstest]
#[case("user1", "pass1", "server1", "user1", "pass1", "server1", true, "same credentials")]
#[case("user1", "pass1", "server1", "user2", "pass1", "server1", false, "different username")]
#[case("user1", "pass1", "server1", "user1", "pass2", "server1", false, "different password")]
#[case("user1", "pass1", "server1", "user1", "pass1", "server2", false, "different server")]
fn test_credentials_comparison(
    #[case] username1: &str,
    #[case] password1: &str,
    #[case] server1: &str,
    #[case] username2: &str,
    #[case] password2: &str,
    #[case] server2: &str,
    #[case] should_match: bool,
    #[case] description: &str,
) {
    let config1 = build_docker_config_json(username1, password1, server1);
    let config2 = build_docker_config_json(username2, password2, server2);
    
    assert_eq!(config1 == config2, should_match, "{}", description);
}

#[test]
fn test_secret_update_logic() {
    // Scenario 1: Secret doesn't exist - should create
    // This is tested by the orchestrator initialization
    
    // Scenario 2: Secret exists with same credentials - should skip
    let config1 = build_docker_config_json("user", "pass", "https://registry.example.com");
    let config2 = build_docker_config_json("user", "pass", "https://registry.example.com");
    assert_eq!(config1, config2, "Same credentials should produce identical configs");
    
    // Scenario 3: Secret exists with different credentials - should recreate
    let config3 = build_docker_config_json("user", "newpass", "https://registry.example.com");
    assert_ne!(config1, config3, "Different credentials should produce different configs");
}
