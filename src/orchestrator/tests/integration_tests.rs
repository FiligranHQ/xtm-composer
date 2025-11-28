use crate::config::settings::Registry;
use crate::config::SecretString;
use crate::orchestrator::registry_resolver::RegistryResolver;
use rstest::rstest;

fn create_test_registry(server: Option<String>, with_auth: bool) -> Registry {
    Registry {
        server,
        username: if with_auth {
            Some(SecretString::new("user".to_string()))
        } else {
            None
        },
        password: if with_auth {
            Some(SecretString::new("pass".to_string()))
        } else {
            None
        },
        email: None,
    }
}

#[test]
fn test_resolve_and_credentials_flow() {
    // Test that the resolution of image works with credentials generation
    let registry = create_test_registry(Some("registry.io".to_string()), true);
    let resolver = RegistryResolver::new(registry);

    // 1. Resolve namespaced image
    let resolved = resolver.resolve_image("org/myapp:v1.0").unwrap();
    assert_eq!(resolved.full_name, "registry.io/org/myapp:v1.0");
    assert!(resolved.needs_auth);

    // 2. Get credentials
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_some());

    let auth = creds.unwrap();
    assert_eq!(auth.username, Some("user".to_string()));
    assert_eq!(auth.password, Some("pass".to_string()));
    assert_eq!(auth.serveraddress, Some("registry.io".to_string()));
}

#[test]
fn test_resolve_and_kube_secret_flow() {
    // Test that image resolution works with Kubernetes secret generation
    let registry = create_test_registry(Some("registry.io".to_string()), true);
    let resolver = RegistryResolver::new(registry);

    // 1. Resolve image
    let resolved = resolver.resolve_image("team/myapp:v2.0").unwrap();
    assert_eq!(resolved.full_name, "registry.io/team/myapp:v2.0");
    assert!(resolved.needs_auth);

    // 2. Get Kubernetes secret data
    let secret_data = resolver.get_kube_secret_data();
    assert!(secret_data.is_ok());

    let (username, password, server) = secret_data.unwrap();
    assert_eq!(username, "user");
    assert_eq!(password, "pass");
    assert_eq!(server, "registry.io");
}

#[test]
fn test_registry_normalization_in_settings() {
    // Test that normalization works correctly in settings
    let mut registry = Registry {
        server: None,
        username: Some(SecretString::new("user".to_string())),
        password: Some(SecretString::new("pass".to_string())),
        email: None,
    };

    registry = registry.normalize();
    assert_eq!(registry.server, Some("docker.io".to_string()));

    // After normalization, should work with resolver
    let resolver = RegistryResolver::new(registry);
    let result = resolver.resolve_image("myapp:latest").unwrap();
    assert_eq!(result.registry_server, Some("docker.io".to_string()));
}

#[rstest]
#[case("org/app1:v1", "registry.io/org/app1:v1")]
#[case("org/app2:v2", "registry.io/org/app2:v2")]
#[case("team/project/app3:latest", "registry.io/team/project/app3:latest")]
fn test_multiple_image_resolutions(
    #[case] input: &str,
    #[case] expected: &str,
) {
    // Test resolving multiple images with the same registry config
    let registry = create_test_registry(Some("registry.io".to_string()), true);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image(input).unwrap();
    assert_eq!(result.full_name, expected);
    assert!(result.needs_auth);
}


#[test]
fn test_credentials_consistency_across_operations() {
    // Ensure credentials are consistent across different operations
    let registry = create_test_registry(Some("registry.example.com".to_string()), true);
    let resolver = RegistryResolver::new(registry);

    // Get Docker credentials
    let docker_creds = resolver.get_docker_credentials().unwrap().unwrap();

    // Get Kubernetes secret data
    let (kube_user, kube_pass, kube_server) = resolver.get_kube_secret_data().unwrap();

    // Should be consistent
    assert_eq!(docker_creds.username, Some(kube_user));
    assert_eq!(docker_creds.password, Some(kube_pass));
    assert_eq!(docker_creds.serveraddress, Some(kube_server));
}

#[test]
fn test_no_credentials_flow() {
    // Test complete flow when no credentials are configured
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);

    // Should resolve image
    let result = resolver.resolve_image("nginx:latest").unwrap();
    assert!(!result.needs_auth);

    // Should return None for credentials
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_none());

    // Should fail for Kubernetes secret data
    let secret_result = resolver.get_kube_secret_data();
    assert!(secret_result.is_err());
}


#[test]
fn test_digest_based_image_with_credentials() {
    // Test that digest-based images work with credentials
    let registry = create_test_registry(Some("registry.io".to_string()), true);
    let resolver = RegistryResolver::new(registry);

    let image =
        "myorg/app@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image).unwrap();

    assert!(result.full_name.contains("@sha256:"));
    assert!(result.needs_auth);

    // Should still be able to get credentials
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_some());
}
