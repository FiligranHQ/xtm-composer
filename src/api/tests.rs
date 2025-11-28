use super::*;

#[test]
fn test_env_value_secret_debug_redacted() {
    let secret = EnvValue::Secret(SecretString::new("password123".to_string()));
    let debug_output = format!("{:?}", secret);
    assert!(debug_output.contains("***REDACTED***"));
    assert!(!debug_output.contains("password123"));
}

#[test]
fn test_env_value_public_visible() {
    let public = EnvValue::Public("public_value".to_string());
    let debug_output = format!("{:?}", public);
    assert!(debug_output.contains("public_value"));
}

#[test]
fn test_env_value_as_str() {
    let public = EnvValue::Public("public_value".to_string());
    let secret = EnvValue::Secret(SecretString::new("secret_value".to_string()));
    
    assert_eq!(public.as_str(), "public_value");
    assert_eq!(secret.as_str(), "secret_value");
}

#[test]
fn test_env_value_is_sensitive() {
    let public = EnvValue::Public("value".to_string());
    let secret = EnvValue::Secret(SecretString::new("secret".to_string()));
    
    assert!(!public.is_sensitive());
    assert!(secret.is_sensitive());
}

#[test]
fn test_env_variable_mixed_types() {
    let public_var = EnvVariable {
        key: "PUBLIC_KEY".to_string(),
        value: EnvValue::Public("value".to_string()),
    };
    
    let secret_var = EnvVariable {
        key: "SECRET_KEY".to_string(),
        value: EnvValue::Secret(SecretString::new("secret".to_string())),
    };
    
    assert!(!public_var.value.is_sensitive());
    assert!(secret_var.value.is_sensitive());
    
    // Test debug output masks secrets
    let debug = format!("{:?}", secret_var);
    assert!(debug.contains("***REDACTED***"));
    assert!(!debug.contains("secret"));
}

#[test]
fn test_env_value_serialization() {
    use serde_json;
    
    let public = EnvValue::Public("test_value".to_string());
    let public_json = serde_json::to_string(&public).unwrap();
    assert_eq!(public_json, r#""test_value""#);
    
    // Secret serializes as REDACTED for safety
    let secret = EnvValue::Secret(SecretString::new("secret_pass".to_string()));
    let secret_json = serde_json::to_string(&secret).unwrap();
    assert_eq!(secret_json, r#""***REDACTED***""#);
    assert!(!secret_json.contains("secret_pass"));
}

#[test]
fn test_api_contract_config_types() {
    let public_config = ApiContractConfig {
        key: "PUBLIC_VAR".to_string(),
        value: EnvValue::Public("public_value".to_string()),
    };
    
    let secret_config = ApiContractConfig {
        key: "SECRET_VAR".to_string(),
        value: EnvValue::Secret(SecretString::new("secret_value".to_string())),
    };
    
    assert!(!public_config.value.is_sensitive());
    assert_eq!(public_config.value.as_str(), "public_value");
    
    assert!(secret_config.value.is_sensitive());
    assert_eq!(secret_config.value.as_str(), "secret_value");
    
    // Verify debug output
    let debug = format!("{:?}", secret_config);
    assert!(debug.contains("***REDACTED***"));
    assert!(!debug.contains("secret_value"));
}

#[test]
fn test_container_envs_creates_correct_types() {
    // Create a test connector with mixed sensitive/public configs
    let connector = ApiConnector {
        id: "test-id".to_string(),
        name: "test-connector".to_string(),
        image: "test/image".to_string(),
        contract_hash: "hash123".to_string(),
        current_status: None,
        requested_status: "starting".to_string(),
        contract_configuration: vec![
            ApiContractConfig {
                key: "PUBLIC_VAR".to_string(),
                value: EnvValue::Public("public_value".to_string()),
            },
            ApiContractConfig {
                key: "SECRET_VAR".to_string(),
                value: EnvValue::Secret(SecretString::new("secret_value".to_string())),
            },
        ],
    };
    
    let envs = connector.container_envs();
    
    // Should have 4 variables: 2 from config + OPENCTI_URL + OPENCTI_CONFIG_HASH
    assert_eq!(envs.len(), 4);
    
    // Find each variable and verify types
    let public_var = envs.iter().find(|e| e.key == "PUBLIC_VAR").unwrap();
    assert!(!public_var.value.is_sensitive());
    assert_eq!(public_var.value.as_str(), "public_value");
    
    let secret_var = envs.iter().find(|e| e.key == "SECRET_VAR").unwrap();
    assert!(secret_var.value.is_sensitive());
    assert_eq!(secret_var.value.as_str(), "secret_value");
    
    // Verify added environment variables are public
    let opencti_url = envs.iter().find(|e| e.key == "OPENCTI_URL").unwrap();
    assert!(!opencti_url.value.is_sensitive());
    
    let config_hash = envs.iter().find(|e| e.key == "OPENCTI_CONFIG_HASH").unwrap();
    assert!(!config_hash.value.is_sensitive());
    assert_eq!(config_hash.value.as_str(), "hash123");
}

#[test]
fn test_env_value_clone() {
    let public = EnvValue::Public("test".to_string());
    let public_clone = public.clone();
    assert_eq!(public.as_str(), public_clone.as_str());
    
    let secret = EnvValue::Secret(SecretString::new("secret".to_string()));
    let secret_clone = secret.clone();
    assert_eq!(secret.as_str(), secret_clone.as_str());
}
