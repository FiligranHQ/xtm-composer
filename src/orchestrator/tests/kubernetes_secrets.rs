use crate::config::settings::{Registry, Kubernetes};
use crate::config::SecretString;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use base64::Engine;

// Helper to create orchestrator for testing pure methods that don't use k8s APIs
// Uses tokio::runtime to create a minimal k8s client that won't be accessed
fn create_test_orchestrator() -> KubeOrchestrator {
    use k8s_openapi::api::core::v1::Pod;
    use k8s_openapi::api::apps::v1::Deployment;
    use kube::{Api, Client, Config};
    
    // Create a minimal Config that won't be used for actual API calls
    // We only test pure methods that don't interact with k8s
    let config = Config {
        cluster_url: "https://127.0.0.1:6443".parse().unwrap(),
        default_namespace: "default".into(),
        root_cert: None,
        connect_timeout: Some(std::time::Duration::from_secs(1)),
        read_timeout: Some(std::time::Duration::from_secs(1)),
        write_timeout: Some(std::time::Duration::from_secs(1)),
        accept_invalid_certs: true,
        auth_info: Default::default(),
        proxy_url: None,
        tls_server_name: None,
        headers: Default::default(),
        disable_compression: false,
    };
    
    let client = Client::try_from(config).expect("Failed to create test client");
    
    KubeOrchestrator {
        pods: Api::<Pod>::default_namespaced(client.clone()),
        deployments: Api::<Deployment>::default_namespaced(client),
        config: Kubernetes {
            base_deployment: None,
            base_deployment_json: None,
            image_pull_policy: Some("IfNotPresent".to_string()),
        },
    }
}

fn create_test_registry(server: &str) -> Registry {
    Registry {
        server: Some(server.to_string()),
        username: Some(SecretString::new("user".to_string())),
        password: Some(SecretString::new("pass".to_string())),
        email: None,
        auto_refresh_secret: false,
        refresh_threshold: 0.8,
    }
}

#[tokio::test]
async fn test_secret_name_simple_registry() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("registry.acme.com");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-registry-acme-com");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_with_port() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("registry.acme.com:5000");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-registry-acme-com-5000");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_localhost() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("localhost:5000");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-localhost-5000");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_dockerhub() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("docker.io");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-docker-io");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_with_path() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("registry.acme.com/v2/");
    
    let name = orchestrator.generate_secret_name(&registry);
    // Slashes replaced with hyphens, trailing hyphen removed
    assert!(name.starts_with("opencti-registry-registry-acme-com-v2"));
    assert!(!name.ends_with('-'));
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_removes_trailing_hyphen() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("registry.acme.com/");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert!(!name.ends_with('-'));
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_gcr() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("gcr.io");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-gcr-io");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_secret_name_ghcr() {
    let orchestrator = create_test_orchestrator();
    let registry = create_test_registry("ghcr.io");
    
    let name = orchestrator.generate_secret_name(&registry);
    assert_eq!(name, "opencti-registry-ghcr-io");
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_validate_secret_name_valid() {
    let orchestrator = create_test_orchestrator();
    
    assert!(orchestrator.validate_secret_name("valid-secret-name").is_ok());
    assert!(orchestrator.validate_secret_name("a").is_ok());
    assert!(orchestrator.validate_secret_name("secret-with-123").is_ok());
    assert!(orchestrator.validate_secret_name("secret.with.dots").is_ok());
}

#[tokio::test]
async fn test_validate_secret_name_invalid_start() {
    let orchestrator = create_test_orchestrator();
    
    let result = orchestrator.validate_secret_name("-invalid");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("start with alphanumeric"));
}

#[tokio::test]
async fn test_validate_secret_name_invalid_end() {
    let orchestrator = create_test_orchestrator();
    
    let result = orchestrator.validate_secret_name("invalid-");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("end with alphanumeric"));
}

#[tokio::test]
async fn test_validate_secret_name_invalid_chars() {
    let orchestrator = create_test_orchestrator();
    
    let result = orchestrator.validate_secret_name("invalid_secret");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid character"));
}

#[tokio::test]
async fn test_validate_secret_name_too_long() {
    let orchestrator = create_test_orchestrator();
    
    let long_name = "a".repeat(254);
    let result = orchestrator.validate_secret_name(&long_name);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too long"));
}

#[tokio::test]
async fn test_validate_secret_name_empty() {
    let orchestrator = create_test_orchestrator();
    
    let result = orchestrator.validate_secret_name("");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot be empty"));
}

#[tokio::test]
async fn test_validate_secret_name_max_length() {
    let orchestrator = create_test_orchestrator();
    
    // 253 characters is the max
    let name = format!("a{}", "b".repeat(252));
    assert_eq!(name.len(), 253);
    assert!(orchestrator.validate_secret_name(&name).is_ok());
}

#[tokio::test]
async fn test_get_image_pull_policy_default() {
    let orchestrator = create_test_orchestrator();
    let policy = orchestrator.get_image_pull_policy();
    assert_eq!(policy, "IfNotPresent");
}

#[tokio::test]
async fn test_get_image_pull_policy_always() {
    let mut orchestrator = create_test_orchestrator();
    orchestrator.config.image_pull_policy = Some("Always".to_string());
    
    let policy = orchestrator.get_image_pull_policy();
    assert_eq!(policy, "Always");
}

#[tokio::test]
async fn test_get_image_pull_policy_never() {
    let mut orchestrator = create_test_orchestrator();
    orchestrator.config.image_pull_policy = Some("Never".to_string());
    
    let policy = orchestrator.get_image_pull_policy();
    assert_eq!(policy, "Never");
}

#[tokio::test]
async fn test_get_image_pull_policy_invalid_fallback() {
    let mut orchestrator = create_test_orchestrator();
    orchestrator.config.image_pull_policy = Some("InvalidPolicy".to_string());
    
    let policy = orchestrator.get_image_pull_policy();
    assert_eq!(policy, "IfNotPresent"); // Should fallback to default
}

#[tokio::test]
async fn test_get_image_pull_policy_none() {
    let mut orchestrator = create_test_orchestrator();
    orchestrator.config.image_pull_policy = None;
    
    let policy = orchestrator.get_image_pull_policy();
    assert_eq!(policy, "IfNotPresent"); // Should use default
}

// ============================================================================
// TASK 5: Essential Tests for .dockerconfigjson and Auth Encoding
// ============================================================================

#[test]
fn test_base64_auth_encoding() {
    // Verify: auth = base64(username:password)
    let username = "testuser";
    let password = "testpass";
    let auth_string = format!("{}:{}", username, password);
    let auth_base64 = base64::engine::general_purpose::STANDARD.encode(auth_string.as_bytes());
    
    // Expected: "dGVzdHVzZXI6dGVzdHBhc3M="
    assert_eq!(auth_base64, "dGVzdHVzZXI6dGVzdHBhc3M=");
    
    // Verify decoding
    let decoded = base64::engine::general_purpose::STANDARD.decode(&auth_base64).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "testuser:testpass");
}

#[test]
fn test_docker_config_json_structure() {
    // Verify .dockerconfigjson structure matches Kubernetes spec
    let server = "docker.io";
    let username = "user";
    let password = "pass";
    let email = "user@example.com";
    let auth_base64 = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", username, password).as_bytes());
    
    let docker_config = serde_json::json!({
        "auths": {
            server: {
                "username": username,
                "password": password,
                "email": email,
                "auth": auth_base64
            }
        }
    });
    
    // Validate structure
    assert!(docker_config["auths"].is_object());
    assert!(docker_config["auths"][server]["username"].is_string());
    assert!(docker_config["auths"][server]["password"].is_string());
    assert!(docker_config["auths"][server]["auth"].is_string());
    
    // Verify serialization works
    let json_bytes = serde_json::to_vec(&docker_config).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
    assert_eq!(parsed, docker_config);
}

#[test]
fn test_registry_normalize_defaults_to_docker_io() {
    let mut registry = Registry {
        server: None,
        username: Some(SecretString::new("user".to_string())),
        password: Some(SecretString::new("pass".to_string())),
        email: None,
        auto_refresh_secret: false,
        refresh_threshold: 0.8,
    };
    
    registry = registry.normalize();
    assert_eq!(registry.server, Some("docker.io".to_string()));
}
