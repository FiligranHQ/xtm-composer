use crate::config::settings::Registry;
use bollard::auth::DockerCredentials;
use oci_distribution::Reference;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub enum RegistryError {
    NoConfig,
    AuthenticationFailed(String),
    ImageResolutionFailed(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoConfig => write!(f, "No registry configuration found"),
            Self::AuthenticationFailed(msg) => write!(f, "Registry authentication failed: {}", msg),
            Self::ImageResolutionFailed(msg) => write!(f, "Image resolution failed: {}", msg),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Resolved image information
#[derive(Debug, Clone)]
pub struct ResolvedImage {
    pub full_name: String,
    pub registry_server: Option<String>,
    pub needs_auth: bool,
}

/// Registry resolver - builds credentials on demand (no caching needed)
pub struct RegistryResolver {
    registry_config: Option<Registry>,
}

impl Clone for RegistryResolver {
    fn clone(&self) -> Self {
        Self {
            registry_config: self.registry_config.clone(),
        }
    }
}

impl RegistryResolver {
    pub fn new(registry_config: Option<Registry>) -> Self {
        // Registry config should always be Some after normalization
        if registry_config.is_none() {
            warn!("Registry config is None - this should not happen after config normalization");
        }
        Self {
            registry_config,
        }
    }

    /// Resolve image name with registry prefix if needed.
    /// Uses oci-distribution to parse OCI references and handle all registry formats.
    pub fn resolve_image(&self, base_image: &str) -> Result<ResolvedImage, RegistryError> {
        if base_image.is_empty() || base_image.contains(char::is_whitespace) {
            return Err(RegistryError::ImageResolutionFailed(
                "Invalid image name: must not be empty or contain whitespace".to_string(),
            ));
        }
        
        let image_ref = Reference::try_from(base_image)
            .map_err(|e| RegistryError::ImageResolutionFailed(
                format!("Invalid OCI image reference '{}': {}", base_image, e)
            ))?;
        
        let config = self.registry_config.as_ref()
            .ok_or(RegistryError::NoConfig)?;
        
        let configured_registry = config.server.as_ref()
            .expect("Registry server should be normalized during config load");
        
        let parsed_registry = image_ref.registry();
        let is_default_dockerhub = parsed_registry == "docker.io" || 
                                   parsed_registry == "index.docker.io";
        
        let should_apply_custom_registry = is_default_dockerhub 
            && configured_registry != "docker.io" 
            && configured_registry != "index.docker.io";
        
        let full_name = if should_apply_custom_registry {
            let repository = image_ref.repository();
            let repository_clean = repository.strip_prefix("library/").unwrap_or(repository);
            
            let tag_or_digest = if let Some(digest) = image_ref.digest() {
                format!("@{}", digest)
            } else {
                format!(":{}", image_ref.tag().unwrap_or("latest"))
            };
            
            let resolved = format!(
                "{}/{}{}", 
                configured_registry.trim_end_matches('/'),
                repository_clean,
                tag_or_digest
            );
            
            debug!(
                base_image = base_image,
                resolved = resolved,
                parsed_registry = parsed_registry,
                configured_registry = configured_registry,
                "Applied custom registry prefix"
            );
            
            resolved
        } else if is_default_dockerhub {
            let repository = image_ref.repository();
            
            // Strip library/ only for single-name images to preserve explicit "library/nginx"
            let repository_clean = if !base_image.contains('/') {
                repository.strip_prefix("library/").unwrap_or(repository)
            } else {
                repository
            };
            
            let tag_or_digest = if let Some(digest) = image_ref.digest() {
                format!("@{}", digest)
            } else {
                format!(":{}", image_ref.tag().unwrap_or("latest"))
            };
            
            let resolved = format!(
                "{}/{}{}", 
                configured_registry.trim_end_matches('/'),
                repository_clean,
                tag_or_digest
            );
            
            debug!(
                base_image = base_image,
                resolved = resolved,
                parsed_registry = parsed_registry,
                configured_registry = configured_registry,
                repository_clean = repository_clean,
                "Docker Hub with library/ handling"
            );
            
            resolved
        } else {
            let preserved = image_ref.whole();
            
            debug!(
                base_image = base_image,
                resolved = preserved,
                parsed_registry = parsed_registry,
                configured_registry = configured_registry,
                "Preserved explicit registry"
            );
            
            preserved
        };
        
        let needs_auth = config.username.is_some() && config.password.is_some();
        
        info!(
            base_image = base_image,
            resolved_image = full_name,
            parsed_registry = parsed_registry,
            configured_registry = configured_registry,
            needs_auth = needs_auth,
            applied_custom_registry = should_apply_custom_registry,
            "Image resolution complete"
        );
        
        Ok(ResolvedImage {
            full_name,
            registry_server: Some(configured_registry.to_string()),
            needs_auth,
        })
    }

    /// Get Docker credentials (builds on demand - very fast, no cache needed)
    pub fn get_docker_credentials(&self) -> Result<Option<DockerCredentials>, RegistryError> {
        let config = self.registry_config.as_ref().ok_or(RegistryError::NoConfig)?;

        if config.username.is_none() || config.password.is_none() {
            return Ok(None);
        }

        Ok(Some(self.build_credentials(config)))
    }

    /// Create Kubernetes imagePullSecret data (no cache needed - secret persists)
    pub fn get_kube_secret_data(&self) -> Result<(String, String, String), RegistryError> {
        let config = self.registry_config.as_ref().ok_or(RegistryError::NoConfig)?;

        let username = config.username.as_ref()
            .ok_or_else(|| RegistryError::AuthenticationFailed("Missing username".to_string()))?
            .expose_secret()
            .to_string();
        let password = config.password.as_ref()
            .ok_or_else(|| RegistryError::AuthenticationFailed("Missing password".to_string()))?
            .expose_secret()
            .to_string();
        let server = config.server.as_ref()
            .ok_or_else(|| RegistryError::AuthenticationFailed("Missing server".to_string()))?
            .clone();

        Ok((username, password, server))
    }

    /// Get registry server if configured
    pub fn get_registry_server(&self) -> Option<String> {
        self.registry_config
            .as_ref()
            .and_then(|c| c.server.clone())
    }

    fn build_credentials(&self, config: &Registry) -> DockerCredentials {
        DockerCredentials {
            username: config.username.as_ref().map(|s| s.expose_secret().to_string()),
            password: config.password.as_ref().map(|s| s.expose_secret().to_string()),
            auth: None,
            email: config.email.clone(),
            serveraddress: config.server.clone(),
            identitytoken: None,
            registrytoken: None,
        }
    }
}
