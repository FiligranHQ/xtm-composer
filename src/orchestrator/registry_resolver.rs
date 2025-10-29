use crate::config::settings::Registry;
use crate::orchestrator::registry_cache::REGISTRY_AUTH_CACHE;
use bollard::auth::DockerCredentials;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

#[derive(Debug)]
pub enum RegistryError {
    NoConfig,
    InvalidConfig(String),
    AuthenticationFailed(String),
    ImageResolutionFailed(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoConfig => write!(f, "No registry configuration found"),
            Self::InvalidConfig(msg) => write!(f, "Invalid registry configuration: {}", msg),
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

/// Registry resolver with authentication caching
pub struct RegistryResolver {
    registry_config: Option<Registry>,
}

impl RegistryResolver {
    pub fn new(registry_config: Option<Registry>) -> Self {
        Self { registry_config }
    }

    /// Resolve image name with registry prefix if needed
    pub fn resolve_image(&self, base_image: &str) -> Result<ResolvedImage, RegistryError> {
        // Validate image name format
        if base_image.is_empty() {
            return Err(RegistryError::ImageResolutionFailed(
                "Image name cannot be empty".to_string(),
            ));
        }

        // Check for invalid characters
        if base_image.contains(' ') || base_image.contains('\t') || base_image.contains('\n') {
            return Err(RegistryError::ImageResolutionFailed(
                "Image name contains invalid whitespace characters".to_string(),
            ));
        }

        let registry_config = match &self.registry_config {
            Some(config) => config,
            None => {
                // No registry config, use image as-is (Docker Hub)
                return Ok(ResolvedImage {
                    full_name: base_image.to_string(),
                    registry_server: None,
                    needs_auth: false,
                });
            }
        };

        let registry_server = match &registry_config.server {
            Some(server) => server.clone(),
            None => {
                return Err(RegistryError::ImageResolutionFailed(
                    "Registry server not configured".to_string(),
                ));
            }
        };

        // Check if image already has a registry prefix
        let needs_prefix = if let Some(first_slash_pos) = base_image.find('/') {
            // Check if there's a dot before the first slash (indicating a registry)
            !base_image[..first_slash_pos].contains('.')
        } else {
            // No slash at all, it's just an image name
            true
        };

        let full_image_name = if needs_prefix {
            let trimmed_server = registry_server.trim_end_matches('/');
            format!("{}/{}", trimmed_server, base_image)
        } else {
            base_image.to_string()
        };

        debug!(
            base_image = base_image,
            resolved_image = full_image_name,
            registry = registry_server,
            needs_prefix = needs_prefix,
            "Resolved image name"
        );

        Ok(ResolvedImage {
            full_name: full_image_name,
            registry_server: Some(registry_server),
            needs_auth: self.has_credentials(),
        })
    }

    /// Get authenticated credentials with caching and retry logic
    pub async fn get_credentials(&self, registry_server: &str) -> Result<DockerCredentials, RegistryError> {
        let config = self.registry_config.as_ref().ok_or(RegistryError::NoConfig)?;

        // Check cache first
        if REGISTRY_AUTH_CACHE.is_auth_valid(registry_server).await {
            debug!(registry = registry_server, "Using cached authentication");
            return self.build_credentials(config);
        }

        // Get retry configuration
        let retry_attempts = config.retry_attempts;
        let retry_delay = std::time::Duration::from_secs(config.retry_delay);

        // Attempt authentication with retries
        for attempt in 1..=retry_attempts {
            match self.authenticate_with_registry(config, registry_server).await {
                Ok(credentials) => {
                    // Cache successful authentication
                    let ttl = std::time::Duration::from_secs(config.token_ttl);
                    
                    REGISTRY_AUTH_CACHE
                        .cache_auth(registry_server.to_string(), Some(ttl))
                        .await;

                    info!(
                        registry = registry_server,
                        ttl = ?ttl,
                        "Authentication successful and cached"
                    );

                    return Ok(credentials);
                }
                Err(e) if attempt < retry_attempts => {
                    warn!(
                        registry = registry_server,
                        attempt = attempt,
                        max_attempts = retry_attempts,
                        error = %e,
                        "Authentication failed, retrying in {:?}",
                        retry_delay
                    );
                    sleep(retry_delay).await;
                }
                Err(e) => {
                    error!(
                        registry = registry_server,
                        attempts = retry_attempts,
                        error = %e,
                        "Authentication failed after all retry attempts"
                    );

                    // Invalidate any cached auth on final failure
                    REGISTRY_AUTH_CACHE.invalidate(registry_server).await;
                    return Err(RegistryError::AuthenticationFailed(e.to_string()));
                }
            }
        }

        unreachable!()
    }

    /// Check if registry credentials are configured
    pub fn has_credentials(&self) -> bool {
        self.registry_config
            .as_ref()
            .map(|config| config.username.is_some() && config.password.is_some())
            .unwrap_or(false)
    }

    /// Get registry server URL if configured
    pub fn get_registry_server(&self) -> Option<String> {
        self.registry_config
            .as_ref()
            .and_then(|config| config.server.clone())
    }

    /// Build Docker credentials from configuration
    fn build_credentials(&self, config: &Registry) -> Result<DockerCredentials, RegistryError> {
        Ok(DockerCredentials {
            username: config.username.clone(),
            password: config.password.clone(),
            auth: None,
            email: config.email.clone(),
            serveraddress: config.server.clone(),
            identitytoken: None,
            registrytoken: None,
        })
    }

    /// Authenticate with registry (placeholder for actual auth test)
    async fn authenticate_with_registry(
        &self,
        config: &Registry,
        registry_server: &str,
    ) -> Result<DockerCredentials, Box<dyn std::error::Error + Send + Sync>> {
        // For Docker, we'll build credentials and let bollard handle the auth
        // In a real implementation, you might want to test the auth first
        debug!(registry = registry_server, "Building registry credentials");

        if config.username.is_none() || config.password.is_none() {
            return Err("Missing username or password".into());
        }

        Ok(DockerCredentials {
            username: config.username.clone(),
            password: config.password.clone(),
            auth: None,
            email: config.email.clone(),
            serveraddress: config.server.clone(),
            identitytoken: None,
            registrytoken: None,
        })
    }
}
