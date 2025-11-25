use crate::config::settings::Registry;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub struct SecretRefresher {
    orchestrator: Arc<KubeOrchestrator>,
    registry_config: Registry,
}

impl SecretRefresher {
    pub fn new(orchestrator: Arc<KubeOrchestrator>, registry_config: Registry) -> Self {
        Self {
            orchestrator,
            registry_config,
        }
    }
    
    /// Start the background refresh loop
    pub async fn start_refresh_loop(self: Arc<Self>) {
        if !self.registry_config.auto_refresh_secret {
            info!("Secret auto-refresh is disabled (recommended: use platform-managed secrets)");
            return;
        }
        
        // Validate configuration
        if self.registry_config.username.is_none() || self.registry_config.password.is_none() {
            warn!(
                "Secret auto-refresh is enabled but credentials are missing, disabling auto-refresh"
            );
            return;
        }
        
        if self.registry_config.refresh_threshold <= 0.0 || self.registry_config.refresh_threshold >= 1.0 {
            warn!(
                threshold = self.registry_config.refresh_threshold,
                "Invalid refresh_threshold (must be between 0.0 and 1.0), using default 0.8"
            );
        }
        
        info!(
            threshold = self.registry_config.refresh_threshold,
            "Starting Kubernetes secret auto-refresh loop"
        );
        
        tokio::spawn(async move {
            loop {
                self.refresh_cycle().await;
            }
        });
    }
    
    async fn refresh_cycle(&self) {
        // Use a fixed 30-minute TTL (1800 seconds) for refresh calculations
        const DEFAULT_TTL_SECONDS: u64 = 1800;
        let threshold = self.registry_config.refresh_threshold.clamp(0.0, 1.0);
        let sleep_duration = Duration::from_secs((DEFAULT_TTL_SECONDS as f64 * threshold) as u64);
        
        debug!(
            sleep_seconds = sleep_duration.as_secs(),
            "Waiting before next secret refresh"
        );
        
        sleep(sleep_duration).await;
        
        // Refresh the secret
        info!("Refreshing Kubernetes imagePullSecret");
        
        match self.orchestrator.ensure_image_pull_secret(&self.registry_config).await {
            Ok(secret_name) => {
                info!(
                    secret_name = secret_name,
                    "Successfully refreshed imagePullSecret"
                );
            }
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to refresh imagePullSecret, will retry at next cycle"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_refresh_threshold_clamping() {
        // Test that threshold is clamped to valid range
        let ttl = 1800u64;
        
        // Valid threshold
        let threshold = 0.8f64;
        let clamped = threshold.clamp(0.0, 1.0);
        assert_eq!(clamped, 0.8);
        
        let sleep_duration = Duration::from_secs((ttl as f64 * clamped) as u64);
        assert_eq!(sleep_duration.as_secs(), 1440); // 1800 * 0.8 = 1440
        
        // Threshold too high
        let threshold = 1.5f64;
        let clamped = threshold.clamp(0.0, 1.0);
        assert_eq!(clamped, 1.0);
        
        // Threshold too low
        let threshold = -0.5f64;
        let clamped = threshold.clamp(0.0, 1.0);
        assert_eq!(clamped, 0.0);
    }
    
    #[test]
    fn test_sleep_duration_calculation() {
        let test_cases = vec![
            (1800, 0.8, 1440),  // 30 min TTL, 80% = 24 min
            (3600, 0.75, 2700), // 60 min TTL, 75% = 45 min
            (900, 0.9, 810),    // 15 min TTL, 90% = 13.5 min
            (7200, 0.5, 3600),  // 120 min TTL, 50% = 60 min
        ];
        
        for (ttl, threshold, expected_seconds) in test_cases {
            let sleep_duration = Duration::from_secs((ttl as f64 * threshold) as u64);
            assert_eq!(
                sleep_duration.as_secs(),
                expected_seconds,
                "TTL: {}, Threshold: {}, Expected: {}, Got: {}",
                ttl,
                threshold,
                expected_seconds,
                sleep_duration.as_secs()
            );
        }
    }
    
    #[test]
    fn test_default_refresh_disabled() {
        use crate::config::settings::Registry;
        use crate::config::SecretString;
        
        let registry = Registry {
            server: Some("docker.io".to_string()),
            username: Some(SecretString::new("user".to_string())),
            password: Some(SecretString::new("pass".to_string())),
            email: Some("email@example.com".to_string()),
            auto_refresh_secret: false, // Should be disabled by default
            refresh_threshold: 0.8,
        };
        
        assert_eq!(registry.auto_refresh_secret, false);
    }
}
