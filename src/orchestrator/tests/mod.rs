#[cfg(test)]
mod registry_resolver;

#[cfg(test)]
mod registry_resolver_concurrency;

#[cfg(test)]
mod kubernetes_secrets;

#[cfg(test)]
mod composer_tests;

#[cfg(test)]
mod error_handling_tests;

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

#[cfg(test)]
mod integration_tests {
    use crate::config::settings::Registry;
    use crate::config::SecretString;
    use crate::orchestrator::registry_resolver::RegistryResolver;

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
            auto_refresh_secret: false,
            refresh_threshold: 0.8,
        }
    }

    #[tokio::test]
    async fn test_resolve_and_credentials_flow() {
        // Test that the resolution of image works with credentials generation
        let registry = create_test_registry(Some("registry.io".to_string()), true);
        let resolver = RegistryResolver::new(Some(registry));

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
        let resolver = RegistryResolver::new(Some(registry));

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
            auto_refresh_secret: false,
            refresh_threshold: 0.8,
        };

        registry = registry.normalize();
        assert_eq!(registry.server, Some("docker.io".to_string()));

        // After normalization, should work with resolver
        let resolver = RegistryResolver::new(Some(registry));
        let result = resolver.resolve_image("myapp:latest").unwrap();
        assert_eq!(result.registry_server, Some("docker.io".to_string()));
    }

    #[test]
    fn test_multiple_image_resolutions() {
        // Test resolving multiple images with the same registry config
        let registry = create_test_registry(Some("registry.io".to_string()), true);
        let resolver = RegistryResolver::new(Some(registry));

        let images = vec![
            ("org/app1:v1", "registry.io/org/app1:v1"),
            ("org/app2:v2", "registry.io/org/app2:v2"),
            ("team/project/app3:latest", "registry.io/team/project/app3:latest"),
        ];

        for (input, expected) in images {
            let result = resolver.resolve_image(input).unwrap();
            assert_eq!(result.full_name, expected);
            assert!(result.needs_auth);
        }
    }

    #[test]
    fn test_mixed_qualified_and_unqualified_images() {
        // Test that qualified images are preserved while namespaced images get prefixed
        let registry = create_test_registry(Some("registry.io".to_string()), false);
        let resolver = RegistryResolver::new(Some(registry));

        // Namespaced image should get registry prefix
        let namespaced = resolver.resolve_image("org/myapp:v1").unwrap();
        assert_eq!(namespaced.full_name, "registry.io/org/myapp:v1");

        // Qualified image should be preserved
        let qualified = resolver.resolve_image("gcr.io/project/app:v1").unwrap();
        assert_eq!(qualified.full_name, "gcr.io/project/app:v1");
    }

    #[tokio::test]
    async fn test_credentials_consistency_across_operations() {
        // Ensure credentials are consistent across different operations
        let registry = create_test_registry(Some("registry.example.com".to_string()), true);
        let resolver = RegistryResolver::new(Some(registry));

        // Get Docker credentials
        let docker_creds = resolver.get_docker_credentials().unwrap().unwrap();

        // Get Kubernetes secret data
        let (kube_user, kube_pass, kube_server) = resolver.get_kube_secret_data().unwrap();

        // Should be consistent
        assert_eq!(docker_creds.username, Some(kube_user));
        assert_eq!(docker_creds.password, Some(kube_pass));
        assert_eq!(docker_creds.serveraddress, Some(kube_server));
    }

    #[tokio::test]
    async fn test_refresh_configuration_with_resolver() {
        // Test that refresh configuration doesn't interfere with resolver operations
        let mut registry = create_test_registry(Some("registry.io".to_string()), true);
        registry.auto_refresh_secret = true;
        registry.refresh_threshold = 0.9;

        let resolver = RegistryResolver::new(Some(registry));

        // Should still resolve namespaced images correctly
        let result = resolver.resolve_image("org/myapp:v1").unwrap();
        assert_eq!(result.full_name, "registry.io/org/myapp:v1");

        // Should still generate credentials correctly
        let creds = resolver.get_docker_credentials().unwrap();
        assert!(creds.is_some());
    }

    #[tokio::test]
    async fn test_no_credentials_flow() {
        // Test complete flow when no credentials are configured
        let registry = create_test_registry(Some("docker.io".to_string()), false);
        let resolver = RegistryResolver::new(Some(registry));

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
    fn test_localhost_registry_integration() {
        // Test localhost registry with full integration flow
        let registry = create_test_registry(Some("docker.io".to_string()), false);
        let resolver = RegistryResolver::new(Some(registry));

        let result = resolver.resolve_image("localhost:5000/myapp:dev").unwrap();

        // Should preserve localhost
        assert_eq!(result.full_name, "localhost:5000/myapp:dev");
        assert!(!result.needs_auth);
    }

    #[tokio::test]
    async fn test_digest_based_image_with_credentials() {
        // Test that digest-based images work with credentials
        let registry = create_test_registry(Some("registry.io".to_string()), true);
        let resolver = RegistryResolver::new(Some(registry));

        let image =
            "myorg/app@sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let result = resolver.resolve_image(image).unwrap();

        assert!(result.full_name.contains("@sha256:"));
        assert!(result.needs_auth);

        // Should still be able to get credentials
        let creds = resolver.get_docker_credentials().unwrap();
        assert!(creds.is_some());
    }
}
