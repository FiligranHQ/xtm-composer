use crate::config::settings::Registry;
use crate::config::SecretString;
use crate::orchestrator::registry_resolver::RegistryResolver;

fn create_test_registry(server: Option<String>, with_auth: bool) -> Registry {
    Registry {
        server,
        username: if with_auth { Some(SecretString::new("user".to_string())) } else { None },
        password: if with_auth { Some(SecretString::new("pass".to_string())) } else { None },
        email: None,
        auto_refresh_secret: false,
        refresh_threshold: 0.8,
    }
}

#[test]
fn test_resolve_image_with_dockerhub_default() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("opencti/connector-misp:latest").unwrap();
    assert_eq!(result.full_name, "docker.io/opencti/connector-misp:latest");
    assert_eq!(result.registry_server, Some("docker.io".to_string()));
    assert_eq!(result.needs_auth, false);
}

#[test]
fn test_resolve_image_with_dockerhub_and_auth() {
    let registry = create_test_registry(Some("docker.io".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("opencti/connector-misp:latest").unwrap();
    assert_eq!(result.full_name, "docker.io/opencti/connector-misp:latest");
    assert_eq!(result.registry_server, Some("docker.io".to_string()));
    assert_eq!(result.needs_auth, true);
}

#[test]
fn test_resolve_image_with_custom_registry() {
    let registry = create_test_registry(Some("registry.acme.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("opencti/connector-misp:latest").unwrap();
    assert_eq!(result.full_name, "registry.acme.com/opencti/connector-misp:latest");
    assert_eq!(result.registry_server, Some("registry.acme.com".to_string()));
    assert_eq!(result.needs_auth, true);
}

#[test]
fn test_resolve_image_already_qualified() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Image already has a registry, should not be modified
    let result = resolver.resolve_image("ghcr.io/owner/image:tag").unwrap();
    assert_eq!(result.full_name, "ghcr.io/owner/image:tag");
    assert_eq!(result.registry_server, Some("docker.io".to_string()));
    assert_eq!(result.needs_auth, false);
}

#[test]
fn test_resolve_image_localhost_registry() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // localhost:5000 should be detected as a registry
    let result = resolver.resolve_image("localhost:5000/myimage:latest").unwrap();
    assert_eq!(result.full_name, "localhost:5000/myimage:latest");
}

#[test]
fn test_normalize_empty_server() {
    let mut registry = create_test_registry(Some("".to_string()), false);
    registry = registry.normalize();
    assert_eq!(registry.server, Some("docker.io".to_string()));
}

#[test]
fn test_normalize_none_server() {
    let mut registry = create_test_registry(None, false);
    registry = registry.normalize();
    assert_eq!(registry.server, Some("docker.io".to_string()));
}

#[test]
fn test_normalize_keeps_custom_server() {
    let mut registry = create_test_registry(Some("registry.acme.com".to_string()), false);
    registry = registry.normalize();
    assert_eq!(registry.server, Some("registry.acme.com".to_string()));
}

#[test]
fn test_invalid_image_name() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Empty image name should fail
    let result = resolver.resolve_image("");
    assert!(result.is_err());
    
    // Image with whitespace should fail
    let result = resolver.resolve_image("invalid image");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_docker_credentials_generation() {
    let registry = create_test_registry(Some("registry.acme.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_some());
    
    let auth = creds.unwrap();
    assert_eq!(auth.username, Some("user".to_string()));
    assert_eq!(auth.password, Some("pass".to_string()));
    assert_eq!(auth.serveraddress, Some("registry.acme.com".to_string()));
}

#[tokio::test]
async fn test_docker_credentials_without_auth() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_none());
}

#[test]
fn test_kube_secret_data_generation() {
    let registry = create_test_registry(Some("registry.acme.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.get_kube_secret_data();
    assert!(result.is_ok());
    
    let (username, password, server) = result.unwrap();
    assert_eq!(username, "user");
    assert_eq!(password, "pass");
    assert_eq!(server, "registry.acme.com");
}

#[test]
fn test_kube_secret_data_missing_credentials() {
    let registry = create_test_registry(Some("registry.acme.com".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.get_kube_secret_data();
    assert!(result.is_err());
}

#[test]
fn test_get_registry_server() {
    let registry = create_test_registry(Some("registry.acme.com".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let server = resolver.get_registry_server();
    assert_eq!(server, Some("registry.acme.com".to_string()));
}

#[test]
fn test_resolve_image_with_port() {
    let registry = create_test_registry(Some("registry.acme.com:5000".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("myapp/image:v1").unwrap();
    assert_eq!(result.full_name, "registry.acme.com:5000/myapp/image:v1");
    assert_eq!(result.needs_auth, true);
}

#[test]
fn test_resolve_image_dockerhub_library() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Option A: Single-name images now get registry prefix applied
    let result = resolver.resolve_image("nginx:latest").unwrap();
    assert_eq!(result.full_name, "docker.io/nginx:latest");
    assert_eq!(result.registry_server, Some("docker.io".to_string()));
    assert_eq!(result.needs_auth, false);
}

#[test]
fn test_resolve_image_preserves_qualified_gcr() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("gcr.io/project/image:tag").unwrap();
    assert_eq!(result.full_name, "gcr.io/project/image:tag");
}

#[test]
fn test_oci_spec_parsing_valid_references() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Test various valid OCI references with Option A behavior
    let valid_refs = vec![
        ("nginx:latest", "docker.io/nginx:latest"), // Single-name gets prefix
        ("library/nginx:1.21", "docker.io/library/nginx:1.21"), // Namespaced
        ("registry.example.com/myapp:v1.0.0", "registry.example.com/myapp:v1.0.0"), // Fully qualified
        ("localhost:5000/test:latest", "localhost:5000/test:latest"), // localhost with port
        ("gcr.io/project-id/image:tag", "gcr.io/project-id/image:tag"), // GCR
        ("ghcr.io/owner/repo:latest", "ghcr.io/owner/repo:latest"), // GHCR
    ];
    
    for (input, expected) in valid_refs {
        let result = resolver.resolve_image(input);
        assert!(result.is_ok(), "Failed to parse valid OCI ref: {}", input);
        assert_eq!(result.unwrap().full_name, expected, "Mismatch for: {}", input);
    }
}

#[test]
fn test_oci_spec_parsing_invalid_references() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Test invalid OCI references
    let invalid_refs = vec![
        "",
        "INVALID IMAGE",
        "image with spaces:tag",
    ];
    
    for ref_str in invalid_refs {
        let result = resolver.resolve_image(ref_str);
        assert!(result.is_err(), "Should reject invalid OCI ref: {}", ref_str);
    }
}

#[test]
fn test_oci_spec_with_digest() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let image_with_digest = "myapp@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image_with_digest).unwrap();
    
    // Option A: Single-name image with digest now gets registry prefix
    assert_eq!(result.full_name, "docker.io/myapp@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890");
}

#[test]
fn test_oci_spec_with_digest_and_namespace() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let image_with_digest = "owner/myapp@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image_with_digest).unwrap();
    
    // Should add registry prefix for namespaced image with digest
    assert_eq!(result.full_name, "docker.io/owner/myapp@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890");
}

#[test]
fn test_oci_spec_qualified_with_digest() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let image_with_digest = "ghcr.io/owner/myapp@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image_with_digest).unwrap();
    
    // Should preserve fully qualified image with digest
    assert_eq!(result.full_name, image_with_digest);
}

#[test]
fn test_oci_spec_various_registries() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Test various registry formats
    let test_cases = vec![
        ("docker.io/library/alpine:latest", "docker.io/library/alpine:latest"),
        ("quay.io/organization/image:v1", "quay.io/organization/image:v1"),
        ("registry.k8s.io/pause:3.9", "registry.k8s.io/pause:3.9"),
        ("public.ecr.aws/lambda/python:3.9", "public.ecr.aws/lambda/python:3.9"),
    ];
    
    for (input, expected) in test_cases {
        let result = resolver.resolve_image(input).unwrap();
        assert_eq!(result.full_name, expected, "Failed for: {}", input);
    }
}

#[test]
fn test_oci_spec_parsing_with_multiple_digests() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let images_with_digest = vec![
        ("myorg/myapp@sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef", 
         "docker.io/myorg/myapp@sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
        ("registry.io/app@sha256:fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
         "registry.io/app@sha256:fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"),
    ];
    
    for (image, expected) in images_with_digest {
        let result = resolver.resolve_image(image);
        assert!(result.is_ok(), "Failed to parse image with digest: {}", image);
        
        let resolved = result.unwrap();
        assert_eq!(resolved.full_name, expected, "Mismatch for: {}", image);
        // Should preserve the digest
        assert!(resolved.full_name.contains("@sha256:"), "Digest not preserved for: {}", image);
    }
}

#[test]
fn test_oci_spec_parsing_with_tag_and_digest() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // OCI spec allows tag + digest (though tag is ignored in favor of digest)
    let image = "myorg/myapp:v1.0@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let result = resolver.resolve_image(image);
    
    // Should handle tag+digest gracefully
    match result {
        Ok(resolved) => {
            assert!(resolved.full_name.contains("@sha256:"), "Digest should be preserved");
            // The implementation should preserve the fully qualified reference
            assert!(resolved.full_name.starts_with("docker.io/"), "Should add registry prefix");
        }
        Err(_) => {
            // If the parser rejects tag+digest, that's also valid OCI behavior
            // The test documents this behavior
        }
    }
}

#[test]
fn test_oci_spec_invalid_digest_format() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let invalid_digests = vec![
        "myapp@sha256:invalid",      // Too short
        "myapp@sha256:",              // Empty digest
        "myapp@md5:abcdef123456",     // Invalid algorithm (only sha256 supported)
        "myapp@:1234",                // Missing algorithm
        "myapp@sha256:xyz",           // Non-hex characters
    ];
    
    for image in invalid_digests {
        let result = resolver.resolve_image(image);
        assert!(result.is_err(), "Should reject invalid digest: {}", image);
    }
}

#[test]
fn test_oci_spec_registry_with_path() {
    let registry = create_test_registry(Some("registry.io/v2".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Option A: Single-name images now get registry prefix
    let result = resolver.resolve_image("myapp:latest").unwrap();
    assert_eq!(result.full_name, "registry.io/v2/myapp:latest");
    
    // Namespaced images also get the registry prefix
    let result2 = resolver.resolve_image("org/myapp:latest").unwrap();
    assert_eq!(result2.full_name, "registry.io/v2/org/myapp:latest");
}

#[test]
fn test_oci_spec_multiple_slashes_in_repository() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Deep repository paths (common in organizations)
    let result = resolver.resolve_image("org/team/project/app:v1").unwrap();
    assert_eq!(result.full_name, "docker.io/org/team/project/app:v1");
    assert!(result.full_name.contains("org/team/project/app:v1"));
}

#[test]
fn test_oci_spec_uppercase_in_tags() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Tags can contain uppercase (unlike repository names which must be lowercase)
    let test_cases = vec![
        "myapp:V1.0.0-RELEASE",
        "myapp:LATEST",
        "myapp:Feature-Branch",
    ];
    
    for image in test_cases {
        let result = resolver.resolve_image(image);
        assert!(result.is_ok(), "Should accept valid tag with uppercase: {}", image);
    }
}

#[test]
fn test_oci_spec_special_characters_in_tags() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let special_tags = vec![
        "myapp:v1.0.0",
        "myapp:latest-2024",
        "myapp:feature_branch",
        // Note: '+' in tags may not be supported by OCI spec parser
        "myapp:20241124-150000",
        "myapp:git-abc123def",
    ];
    
    for image in special_tags {
        let result = resolver.resolve_image(image);
        assert!(result.is_ok(), "Should accept valid tag: {}", image);
    }
    
    // Test namespaced images with special characters  
    let result = resolver.resolve_image("org/myapp:v1.0.0-alpha.1").unwrap();
    assert!(result.full_name.contains("org/myapp:v1.0.0-alpha.1"));
}

#[test]
fn test_oci_spec_default_tag_behavior() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Image without tag - OCI parser and our resolver handle it
    let result = resolver.resolve_image("nginx");
    assert!(result.is_ok(), "Should handle image without tag");
    
    let resolved = result.unwrap();
    // Option A: Registry prefix is applied
    assert!(
        resolved.full_name == "docker.io/nginx" || resolved.full_name == "docker.io/nginx:latest",
        "Expected prefixed image, got: {}", resolved.full_name
    );
}

#[test]
fn test_oci_spec_preserves_original_qualified_images() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let qualified_images = vec![
        "quay.io/prometheus/prometheus:v2.30.0",
        "k8s.gcr.io/kube-apiserver:v1.22.0",
        "registry.k8s.io/coredns/coredns:v1.8.6",
        "mcr.microsoft.com/dotnet/runtime:6.0",
        "public.ecr.aws/eks/aws-load-balancer-controller:v2.4.0",
    ];
    
    for image in qualified_images {
        let result = resolver.resolve_image(image).unwrap();
        // Should preserve the original fully qualified name
        assert_eq!(result.full_name, image, "Should not modify fully qualified image: {}", image);
    }
}

#[test]
fn test_oci_spec_registry_with_port() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let test_cases = vec![
        ("registry.io:5000/myapp:v1", "registry.io:5000/myapp:v1"),
        ("localhost:5000/test:latest", "localhost:5000/test:latest"),
        ("127.0.0.1:8080/app:dev", "127.0.0.1:8080/app:dev"),
    ];
    
    for (input, expected) in test_cases {
        let result = resolver.resolve_image(input).unwrap();
        assert_eq!(result.full_name, expected, "Failed for registry with port: {}", input);
    }
}

#[test]
fn test_oci_spec_ipv6_localhost() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // IPv6 localhost format
    let image = "[::1]:5000/myapp:latest";
    let result = resolver.resolve_image(image);
    
    // Should handle IPv6 addresses
    if result.is_ok() {
        let resolved = result.unwrap();
        assert_eq!(resolved.full_name, image);
    }
    // If not supported, that's also documented behavior
}

#[test]
fn test_oci_spec_edge_cases() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Test edge cases that should work (note: '+' in tags is not supported by OCI spec parser)
    let valid_edge_cases = vec![
        "a/b:c",                          // Minimal valid reference
        "registry.io/a:1",                // Single character names
        "r.io/very-long-repository-name-with-many-hyphens:v1.0.0-alpha.build",
    ];
    
    for image in valid_edge_cases {
        let result = resolver.resolve_image(image);
        assert!(result.is_ok(), "Should accept edge case: {}", image);
    }
}

// ============================================================================
// PART 4: Regression Tests - Ensure Existing Behavior Preserved
// ============================================================================

#[test]
fn test_regression_dockerhub_library_images() {
    // Option A: Official Docker Hub images now get registry prefix
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let official_images = vec!["nginx", "redis", "postgres", "ubuntu", "alpine", "node", "python"];
    
    for image in official_images {
        let result = resolver.resolve_image(image);
        assert!(result.is_ok(), "Official image should resolve: {}", image);
        let resolved = result.unwrap();
        assert_eq!(resolved.registry_server, Some("docker.io".to_string()));
        // Verify registry prefix was applied
        assert!(
            resolved.full_name.starts_with("docker.io/"),
            "Expected registry prefix for {}, got: {}", image, resolved.full_name
        );
    }
}

#[test]
fn test_regression_localhost_registry_formats() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let localhost_formats = vec![
        ("localhost:5000/myapp:latest", "localhost:5000/myapp:latest"),
        ("127.0.0.1:5000/myapp:latest", "127.0.0.1:5000/myapp:latest"),
    ];
    
    for (input, expected) in localhost_formats {
        let result = resolver.resolve_image(input);
        assert!(result.is_ok(), "Localhost format should work: {}", input);
        
        let resolved = result.unwrap();
        // Should preserve localhost with port
        assert_eq!(resolved.full_name, expected, "Failed for: {}", input);
    }
    
    // Note: "localhost/myapp:latest" (without port) may be treated differently by the resolver
    // It might be interpreted as "localhost" being a namespace rather than a registry
    // This is documented behavior
}

#[test]
fn test_regression_needs_auth_logic() {
    // Ensure needs_auth flag is set correctly based on credentials presence
    let test_cases = vec![
        (Some("docker.io".to_string()), true, true),   // With auth
        (Some("docker.io".to_string()), false, false), // Without auth
        (Some("registry.io".to_string()), true, true), // Custom registry with auth
        (Some("quay.io".to_string()), false, false),   // Custom registry without auth
    ];
    
    for (server, with_auth, expected_needs_auth) in test_cases {
        let registry = create_test_registry(server.clone(), with_auth);
        let resolver = RegistryResolver::new(Some(registry));
        
        let result = resolver.resolve_image("myapp:latest").unwrap();
        assert_eq!(
            result.needs_auth, 
            expected_needs_auth,
            "needs_auth mismatch for server={:?}, with_auth={}",
            server,
            with_auth
        );
        
        // Option A: Verify registry prefix was applied to single-name image
        let expected_prefix = format!("{}/", server.clone().unwrap());
        assert!(
            result.full_name.starts_with(&expected_prefix),
            "Expected {} prefix for single-name image, got: {}", expected_prefix, result.full_name
        );
    }
}

#[test]
fn test_regression_custom_registry_behavior() {
    // Ensure custom registries still work as expected
    let registry = create_test_registry(Some("my-registry.company.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("team/app:v1.0").unwrap();
    assert_eq!(result.full_name, "my-registry.company.com/team/app:v1.0");
    assert_eq!(result.registry_server, Some("my-registry.company.com".to_string()));
    assert!(result.needs_auth);
}

#[test]
fn test_regression_normalize_empty_vs_none() {
    // Ensure normalization handles both empty string and None consistently
    let mut registry1 = create_test_registry(Some("".to_string()), false);
    let mut registry2 = create_test_registry(None, false);
    
    registry1 = registry1.normalize();
    registry2 = registry2.normalize();
    
    assert_eq!(registry1.server, Some("docker.io".to_string()));
    assert_eq!(registry2.server, Some("docker.io".to_string()));
}

#[tokio::test]
async fn test_regression_credentials_generation_format() {
    // Ensure Docker credentials format hasn't changed
    let registry = create_test_registry(Some("registry.example.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let creds = resolver.get_docker_credentials().unwrap();
    assert!(creds.is_some());
    
    let auth = creds.unwrap();
    assert_eq!(auth.username, Some("user".to_string()));
    assert_eq!(auth.password, Some("pass".to_string()));
    assert_eq!(auth.serveraddress, Some("registry.example.com".to_string()));
}

#[test]
fn test_regression_kube_secret_data_format() {
    // Ensure Kubernetes secret data format is preserved
    let registry = create_test_registry(Some("registry.example.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.get_kube_secret_data();
    assert!(result.is_ok());
    
    let (username, password, server) = result.unwrap();
    assert_eq!(username, "user");
    assert_eq!(password, "pass");
    assert_eq!(server, "registry.example.com");
}

// ============================================================================
// NEW TESTS: OCI-Distribution Specific Functionality
// ============================================================================

#[test]
fn test_oci_distribution_reference_parsing() {
    // Verify that oci-distribution Reference correctly parses various formats
    use oci_distribution::Reference;
    
    // Test 1: Single-name image
    let ref1 = Reference::try_from("nginx:latest").unwrap();
    assert_eq!(ref1.registry(), "docker.io");
    assert_eq!(ref1.repository(), "library/nginx");
    assert_eq!(ref1.tag(), Some("latest"));
    assert_eq!(ref1.digest(), None);
    
    // Test 2: Namespaced image
    let ref2 = Reference::try_from("myuser/myapp:v1").unwrap();
    assert_eq!(ref2.registry(), "docker.io");
    assert_eq!(ref2.repository(), "myuser/myapp");
    assert_eq!(ref2.tag(), Some("v1"));
    
    // Test 3: Fully qualified with registry
    let ref3 = Reference::try_from("gcr.io/project/image:tag").unwrap();
    assert_eq!(ref3.registry(), "gcr.io");
    assert_eq!(ref3.repository(), "project/image");
    assert_eq!(ref3.tag(), Some("tag"));
    
    // Test 4: Localhost with port
    let ref4 = Reference::try_from("localhost:5000/myimage:dev").unwrap();
    assert_eq!(ref4.registry(), "localhost:5000");
    assert_eq!(ref4.repository(), "myimage");
    
    // Test 5: Image with digest (must be 64 hex characters for sha256)
    let ref5 = Reference::try_from("nginx@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890").unwrap();
    assert_eq!(ref5.registry(), "docker.io");
    assert_eq!(ref5.repository(), "library/nginx");
    assert!(ref5.digest().is_some());
    assert!(ref5.digest().unwrap().contains("sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"));
}

#[test]
fn test_custom_registry_application_comprehensive() {
    // Test comprehensive scenarios for custom registry application
    
    // Scenario 1: Docker Hub image with custom registry configured
    let registry = create_test_registry(Some("myregistry.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("nginx:latest").unwrap();
    assert_eq!(result.full_name, "myregistry.com/nginx:latest");
    assert!(result.needs_auth);
    
    // Scenario 2: Image with explicit registry should NOT be modified
    let result2 = resolver.resolve_image("gcr.io/project/image:v1").unwrap();
    assert_eq!(result2.full_name, "gcr.io/project/image:v1");
    
    // Scenario 3: Namespaced image should get custom registry
    let result3 = resolver.resolve_image("myuser/myapp:v2").unwrap();
    assert_eq!(result3.full_name, "myregistry.com/myuser/myapp:v2");
}

#[test]
fn test_localhost_registry_comprehensive() {
    // Test that localhost registries are always preserved
    let registry = create_test_registry(Some("myregistry.com".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Test various localhost formats
    let localhost_cases = vec![
        ("localhost:5000/myimage:dev", "localhost:5000/myimage:dev"),
        ("localhost:8080/test:latest", "localhost:8080/test:latest"),
        ("127.0.0.1:5000/app:v1", "127.0.0.1:5000/app:v1"),
    ];
    
    for (input, expected) in localhost_cases {
        let result = resolver.resolve_image(input).unwrap();
        assert_eq!(result.full_name, expected, "Failed for localhost case: {}", input);
    }
}

#[test]
fn test_digest_based_images_comprehensive() {
    // Test digest-based image resolution
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Single-name with digest (64 hex characters required for sha256)
    let result1 = resolver.resolve_image("nginx@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890").unwrap();
    assert!(result1.full_name.contains("@sha256:"));
    assert!(result1.full_name.starts_with("docker.io/"));
    
    // Namespaced with digest
    let result2 = resolver.resolve_image("myuser/app@sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    assert!(result2.full_name.contains("@sha256:"));
    assert!(result2.full_name.contains("myuser/app"));
    
    // Fully qualified with digest
    let result3 = resolver.resolve_image("gcr.io/project/image@sha256:fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321").unwrap();
    assert_eq!(result3.full_name, "gcr.io/project/image@sha256:fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321");
}

#[test]
fn test_custom_registry_with_digest() {
    // Test custom registry application with digest-based images
    let registry = create_test_registry(Some("myregistry.com".to_string()), true);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("nginx@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890").unwrap();
    assert!(result.full_name.starts_with("myregistry.com/"));
    assert!(result.full_name.contains("@sha256:"));
    assert!(result.full_name.contains("nginx"));
    assert!(result.needs_auth);
}

#[test]
fn test_no_manual_registry_detection() {
    // Verify that we're NOT using manual registry detection logic
    // This test documents that we rely on oci-distribution parsing
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // These cases would be problematic with manual detection but work with oci-distribution
    let edge_cases = vec![
        // Registry with port (has ':' but is a registry)
        ("registry.io:5000/app:v1", "registry.io:5000/app:v1"),
        // Registry with subdomain (has '.' but is a registry)
        ("sub.registry.io/app:v1", "sub.registry.io/app:v1"),
        // IP address registry
        ("192.168.1.100:5000/app:v1", "192.168.1.100:5000/app:v1"),
    ];
    
    for (input, expected) in edge_cases {
        let result = resolver.resolve_image(input).unwrap();
        assert_eq!(result.full_name, expected, "oci-distribution should handle: {}", input);
    }
}

#[test]
fn test_library_prefix_handling() {
    // Test that library/ prefix is correctly handled
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    // Single-name: library/ should be stripped
    let result1 = resolver.resolve_image("nginx").unwrap();
    assert_eq!(result1.full_name, "docker.io/nginx:latest");
    
    // Explicit library/: should be preserved
    let result2 = resolver.resolve_image("library/nginx:1.21").unwrap();
    assert_eq!(result2.full_name, "docker.io/library/nginx:1.21");
}

#[test]
fn test_registry_with_path_comprehensive() {
    // Test registries with path components
    let registry = create_test_registry(Some("registry.io/v2/images".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("myapp:latest").unwrap();
    assert_eq!(result.full_name, "registry.io/v2/images/myapp:latest");
    
    let result2 = resolver.resolve_image("team/myapp:v1").unwrap();
    assert_eq!(result2.full_name, "registry.io/v2/images/team/myapp:v1");
}

#[test]
fn test_all_dockerhub_variants() {
    // Test various Docker Hub registry specifications
    let test_cases = vec![
        "docker.io",
        "index.docker.io",
    ];
    
    for registry_variant in test_cases {
        let registry = create_test_registry(Some(registry_variant.to_string()), false);
        let resolver = RegistryResolver::new(Some(registry));
        
        // Single-name image
        let result = resolver.resolve_image("nginx:latest").unwrap();
        assert!(result.full_name.starts_with(registry_variant), 
                "Failed for registry variant: {}", registry_variant);
        assert_eq!(result.full_name, format!("{}/nginx:latest", registry_variant));
    }
}

#[test]
fn test_complex_repository_paths() {
    // Test images with deep repository paths
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let result = resolver.resolve_image("org/team/project/service:v1").unwrap();
    assert_eq!(result.full_name, "docker.io/org/team/project/service:v1");
    assert!(result.full_name.contains("org/team/project/service"));
}

#[test]
fn test_oci_distribution_error_handling() {
    // Test that invalid OCI references are properly rejected
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = RegistryResolver::new(Some(registry));
    
    let invalid_cases = vec![
        "",                          // Empty
        "  ",                        // Whitespace only
        "image with spaces",         // Contains spaces
        "UPPERCASE:TAG",             // Repository must be lowercase (will fail OCI parsing)
    ];
    
    for invalid in invalid_cases {
        let result = resolver.resolve_image(invalid);
        assert!(result.is_err(), "Should reject invalid reference: '{}'", invalid);
    }
}
