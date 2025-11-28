use crate::config::settings::Registry;
use crate::config::SecretString;
use crate::orchestrator::registry_resolver::RegistryResolver;
use rstest::rstest;

fn create_test_registry(server: Option<String>, with_auth: bool) -> Registry {
    Registry {
        server,
        username: if with_auth {
            Some(SecretString::new("testuser".to_string()))
        } else {
            None
        },
        password: if with_auth {
            Some(SecretString::new("testpass".to_string()))
        } else {
            None
        },
        email: None,
    }
}

// ============================================================================
// Core Image Resolution Tests - OpenCTI Real-World Scenarios Only
// ============================================================================

#[rstest]
// Docker Hub default registry
#[case("docker.io", false, "opencti/connector-misp:latest", "docker.io/opencti/connector-misp:latest")]
#[case("docker.io", true, "opencti/connector-misp:latest", "docker.io/opencti/connector-misp:latest")]
// Custom registry
#[case("registry.company.com", true, "opencti/connector-misp:latest", "registry.company.com/opencti/connector-misp:latest")]
#[case("registry.company.com", false, "myorg/connector:v1.0", "registry.company.com/myorg/connector:v1.0")]
// Official Docker images (no slash) - oci-distribution adds "library/" prefix
#[case("docker.io", false, "nginx:latest", "docker.io/library/nginx:latest")]
#[case("docker.io", false, "postgres:15", "docker.io/library/postgres:15")]
// Custom registry with official images
#[case("registry.company.com", true, "nginx:latest", "registry.company.com/library/nginx:latest")]
fn test_basic_image_resolution(
    #[case] registry_server: &str,
    #[case] with_auth: bool,
    #[case] input_image: &str,
    #[case] expected_output: &str,
) {
    let registry = create_test_registry(Some(registry_server.to_string()), with_auth);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image(input_image).unwrap();
    assert_eq!(result.full_name, expected_output);
    assert_eq!(result.registry_server, Some(registry_server.to_string()));
    assert_eq!(result.needs_auth, with_auth);
}

// ============================================================================
// Registry Normalization Tests
// ============================================================================

#[rstest]
#[case(Some(""), "docker.io")]
#[case(None, "docker.io")]
#[case(Some("registry.company.com"), "registry.company.com")]
fn test_registry_normalization(#[case] input: Option<&str>, #[case] expected: &str) {
    let mut registry = create_test_registry(input.map(|s| s.to_string()), false);
    registry = registry.normalize();
    assert_eq!(registry.server, Some(expected.to_string()));
}

// ============================================================================
// Invalid Input Tests
// ============================================================================

#[rstest]
#[case("")]
#[case("image with spaces")]
fn test_invalid_image_names_rejected(#[case] invalid_image: &str) {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image(invalid_image);
    assert!(result.is_err());
}


// ============================================================================
// Authentication Detection Tests
// ============================================================================

#[rstest]
#[case(true, true, true)]
#[case(true, false, false)]
#[case(false, true, false)]
#[case(false, false, false)]
fn test_authentication_detection(
    #[case] has_username: bool,
    #[case] has_password: bool,
    #[case] expected_needs_auth: bool,
) {
    let registry = Registry {
        server: Some("registry.company.com".to_string()),
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
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image("myapp:latest").unwrap();
    assert_eq!(result.needs_auth, expected_needs_auth);
}

// ============================================================================
// Docker Credentials Generation Tests
// ============================================================================

#[rstest]
#[case("registry.company.com", true, Some("testuser"), Some("testpass"), Some("registry.company.com"))]
#[case("docker.io", false, None, None, None)]
fn test_docker_credentials(
    #[case] server: &str,
    #[case] with_auth: bool,
    #[case] expected_user: Option<&str>,
    #[case] expected_pass: Option<&str>,
    #[case] expected_server: Option<&str>,
) {
    let registry = create_test_registry(Some(server.to_string()), with_auth);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.get_docker_credentials();
    assert!(result.is_ok());
    
    let creds = result.unwrap();
    
    if expected_user.is_none() {
        assert!(creds.is_none());
    } else {
        assert!(creds.is_some());
        let auth = creds.unwrap();
        assert_eq!(auth.username, expected_user.map(|s| s.to_string()));
        assert_eq!(auth.password, expected_pass.map(|s| s.to_string()));
        assert_eq!(auth.serveraddress, expected_server.map(|s| s.to_string()));
    }
}

// ============================================================================
// Kubernetes Secret Data Tests
// ============================================================================

#[rstest]
#[case("registry.company.com", true, true, Some("testuser"), Some("testpass"), Some("registry.company.com"))]
#[case("registry.company.com", false, false, None, None, None)]
fn test_kube_secret_data(
    #[case] server: &str,
    #[case] with_auth: bool,
    #[case] should_succeed: bool,
    #[case] expected_user: Option<&str>,
    #[case] expected_pass: Option<&str>,
    #[case] expected_server: Option<&str>,
) {
    let registry = create_test_registry(Some(server.to_string()), with_auth);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.get_kube_secret_data();
    
    if !should_succeed {
        assert!(result.is_err());
        return;
    }
    
    assert!(result.is_ok());
    let (username, password, server) = result.unwrap();
    assert_eq!(username, expected_user.unwrap());
    assert_eq!(password, expected_pass.unwrap());
    assert_eq!(server, expected_server.unwrap());
}

// ============================================================================
// Get Registry Server Tests
// ============================================================================

#[rstest]
#[case("registry.company.com", "registry.company.com")]
#[case("docker.io", "docker.io")]
fn test_get_registry_server(#[case] input: &str, #[case] expected: &str) {
    let registry = create_test_registry(Some(input.to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    let server = resolver.get_registry_server();
    assert_eq!(server, expected.to_string());
}

// ============================================================================
// Tag and Version Format Tests
// ============================================================================

#[rstest]
#[case("myapp:v1.0.0", "docker.io/library/myapp:v1.0.0")] // oci-distribution adds library/ for single-segment names
#[case("myapp:latest", "docker.io/library/myapp:latest")]
#[case("myapp:20241124", "docker.io/library/myapp:20241124")]
#[case("org/myapp:v1.0-alpha", "docker.io/org/myapp:v1.0-alpha")]
#[case("myapp", "docker.io/library/myapp:latest")] // Default to :latest
fn test_tag_formats(#[case] input: &str, #[case] expected: &str) {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image(input).unwrap();
    assert_eq!(result.full_name, expected);
}

// ============================================================================
// Digest Support Tests (for reproducible builds)
// ============================================================================

#[test]
fn test_image_with_digest() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);

    let image = "nginx@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image).unwrap();

    assert!(result.full_name.contains("@sha256:"));
    assert!(result.full_name.starts_with("docker.io/"));
}

// ============================================================================
// Registry with Port Tests (for on-premise installations)
// ============================================================================

#[rstest]
#[case("registry.company.com:5000", "myapp:latest", "registry.company.com:5000/library/myapp:latest")]
#[case("registry.company.com:443", "org/app:v1", "registry.company.com:443/org/app:v1")]
fn test_registry_with_port(
    #[case] registry_server: &str,
    #[case] input_image: &str,
    #[case] expected_output: &str,
) {
    let registry = create_test_registry(Some(registry_server.to_string()), false);
    let resolver = RegistryResolver::new(registry);

    let result = resolver.resolve_image(input_image).unwrap();
    assert_eq!(result.full_name, expected_output);
}

// ============================================================================
// Edge Case Tests - Registry with Trailing Slash
// ============================================================================

#[test]
fn test_registry_with_trailing_slash() {
    let registry = create_test_registry(Some("registry.io/".to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    let result = resolver.resolve_image("myapp:latest").unwrap();
    // Should not create "registry.io//myapp" (library/ is added by oci-distribution)
    assert_eq!(result.full_name, "registry.io/library/myapp:latest");
    assert!(!result.full_name.contains("//"));
}

// ============================================================================
// Edge Case Tests - Image Case Normalization
// ============================================================================

#[test]
fn test_image_case_handling() {
    // OCI spec requires lowercase - uppercase should fail
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    // Test that uppercase is rejected by OCI parser
    let result = resolver.resolve_image("NGINX:LATEST");
    assert!(result.is_err());
    
    // Lowercase should work
    let result = resolver.resolve_image("nginx:latest");
    assert!(result.is_ok());
}

// ============================================================================
// Edge Case Tests - IPv4/IPv6 Registry Addresses
// ============================================================================

#[rstest]
#[case("192.168.1.100:5000", "myapp:latest", "192.168.1.100:5000/library/myapp:latest")]
#[case("10.0.0.1", "nginx:latest", "10.0.0.1/library/nginx:latest")]
#[case("[::1]:5000", "test:v1", "[::1]:5000/library/test:v1")] // IPv6 localhost
fn test_ip_address_registries(
    #[case] registry_server: &str,
    #[case] input_image: &str,
    #[case] expected_output: &str,
) {
    let registry = create_test_registry(Some(registry_server.to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    let result = resolver.resolve_image(input_image).unwrap();
    assert_eq!(result.full_name, expected_output);
}

// ============================================================================
// Edge Case Tests - Very Long Image Names (OCI spec limits)
// ============================================================================

#[test]
fn test_long_image_name() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    // Create a name approaching OCI limits (registry.io + / + repository + : + tag)
    // OCI spec: repository name max 255 chars
    let long_repo = "a".repeat(200);
    let image_name = format!("{}:latest", long_repo);
    
    let result = resolver.resolve_image(&image_name);
    // Should handle long names (OCI library validates this)
    assert!(result.is_ok());
}

#[test]
fn test_very_long_image_name_with_path() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(registry);
    
    // Test with org/repo pattern
    let long_name = format!("myorg/{}", "b".repeat(200));
    let image_name = format!("{}:v1.0", long_name);
    
    let result = resolver.resolve_image(&image_name);
    assert!(result.is_ok());
}

// ============================================================================
// Concurrency Safety Test
// ============================================================================

#[test]
fn test_resolver_is_cloneable() {
    let registry = create_test_registry(Some("docker.io".to_string()), true);
    let resolver1 = RegistryResolver::new(registry);
    let resolver2 = resolver1.clone();

    let result1 = resolver1.resolve_image("nginx:latest").unwrap();
    let result2 = resolver2.resolve_image("nginx:latest").unwrap();

    assert_eq!(result1.full_name, result2.full_name);
    assert_eq!(result1.needs_auth, result2.needs_auth);
}
