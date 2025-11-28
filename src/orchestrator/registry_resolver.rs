use crate::config::settings::Registry;
use bollard::auth::DockerCredentials;
use oci_distribution::Reference;
use tracing::info;

#[derive(Debug)]
pub enum RegistryError {
    AuthenticationFailed(String),
    ImageResolutionFailed(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

impl ResolvedImage {
    fn new(full_name: String) -> Self {
        Self {
            full_name,
            registry_server: None,
            needs_auth: false,
        }
    }
    
    fn with_server(mut self, server: String) -> Self {
        self.registry_server = Some(server);
        self
    }
    
    fn with_auth(mut self, needs_auth: bool) -> Self {
        self.needs_auth = needs_auth;
        self
    }
}

/// Registry resolver - builds credentials on demand (no caching needed)
pub struct RegistryResolver {
    registry_config: Registry,
}

impl Clone for RegistryResolver {
    fn clone(&self) -> Self {
        Self {
            registry_config: self.registry_config.clone(),
        }
    }
}

impl RegistryResolver {
    pub fn new(registry_config: Registry) -> Self {
        Self {
            registry_config,
        }
    }

    /// Extract tag or digest from OCI reference
    fn extract_tag_or_digest(&self, image_ref: &Reference) -> String {
        if let Some(digest) = image_ref.digest() {
            format!("@{}", digest)
        } else {
            format!(":{}", image_ref.tag().unwrap_or("latest"))
        }
    }

    /// Resolve image name with registry prefix.
    /// 
    /// # Assumptions
    /// 
    /// **IMPORTANT**: OpenCTI backend MUST send simple image names (nginx, opencti/connector-misp)
    /// and NEVER fully qualified names (ghcr.io/owner/repo, gcr.io/project/image).
    /// 
    /// This resolver ALWAYS prefixes the configured registry to the image name.
    /// Fully qualified image names are NOT supported by design.
    /// 
    /// Uses oci-distribution to parse OCI references for tag/digest extraction.
    pub fn resolve_image(&self, base_image: &str) -> Result<ResolvedImage, RegistryError> {
        // Validate input
        if base_image.is_empty() || base_image.contains(char::is_whitespace) {
            return Err(RegistryError::ImageResolutionFailed(
                "Invalid image name: must not be empty or contain whitespace".to_string(),
            ));
        }
        
        // Parse with oci-distribution to extract components
        let image_ref = Reference::try_from(base_image)
            .map_err(|e| RegistryError::ImageResolutionFailed(
                format!("Invalid OCI image reference '{}': {}", base_image, e)
            ))?;
        
        let registry = self.registry_config.server.as_ref()
            .expect("Registry server should be normalized during config load");
        
        // Always prefix with configured registry
        let repository = image_ref.repository();
        let tag_or_digest = self.extract_tag_or_digest(&image_ref);
        let full_name = self.build_image_name(registry, repository, &tag_or_digest);
        
        let needs_auth = self.registry_config.username.is_some() && self.registry_config.password.is_some();
        
        info!(
            base_image = base_image,
            resolved_image = full_name,
            registry = registry,
            needs_auth = needs_auth,
            "Image resolution complete"
        );
        
        Ok(ResolvedImage::new(full_name)
            .with_server(registry.to_string())
            .with_auth(needs_auth))
    }

    /// Get Docker credentials (builds on demand - very fast, no cache needed)
    pub fn get_docker_credentials(&self) -> Result<Option<DockerCredentials>, RegistryError> {
        if self.registry_config.username.is_none() || self.registry_config.password.is_none() {
            return Ok(None);
        }

        Ok(Some(self.build_credentials(&self.registry_config)))
    }

    /// Create Kubernetes imagePullSecret data (no cache needed - secret persists)
    pub fn get_kube_secret_data(&self) -> Result<(String, String, String), RegistryError> {
        let username = self.registry_config.username.as_ref()
            .ok_or_else(|| RegistryError::AuthenticationFailed("Missing username".to_string()))?
            .expose_secret()
            .to_string();
        let password = self.registry_config.password.as_ref()
            .ok_or_else(|| RegistryError::AuthenticationFailed("Missing password".to_string()))?
            .expose_secret()
            .to_string();
        
        // Server is guaranteed to be Some after normalize()
        let server = self.registry_config.server.clone()
            .expect("Registry server is always Some after normalize()");

        Ok((username, password, server))
    }

    /// Get registry server (always available after normalization)
    pub fn get_registry_server(&self) -> String {
        self.registry_config.server.clone()
            .expect("Registry server is always Some after normalize()")
    }

    /// Helper to build full image name from components
    fn build_image_name(&self, registry: &str, repository: &str, tag_or_digest: &str) -> String {
        format!(
            "{}/{}{}", 
            registry.trim_end_matches('/'),
            repository,
            tag_or_digest
        )
    }

    fn build_credentials(&self, config: &Registry) -> DockerCredentials {
        DockerCredentials {
            username: config.username.as_ref().map(|s| s.expose_secret().to_string()),
            password: config.password.as_ref().map(|s| s.expose_secret().to_string()),
            auth: None,
            email: config.email.as_ref().map(|s| s.expose_secret().to_string()),
            serveraddress: config.server.clone(),
            identitytoken: None,
            registrytoken: None,
        }
    }
}
