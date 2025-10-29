use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Cached authentication entry with TTL
#[derive(Clone)]
pub struct CachedAuth {
    pub cached_at: Instant,
    pub ttl: Duration,
}

impl CachedAuth {
    pub fn new(ttl: Duration) -> Self {
        Self {
            cached_at: Instant::now(),
            ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
}

/// In-memory authentication cache for registry credentials
pub struct RegistryAuthCache {
    cache: RwLock<HashMap<String, CachedAuth>>,
    default_ttl: Duration,
}

impl RegistryAuthCache {
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            default_ttl,
        }
    }

    /// Check if registry has valid cached authentication
    pub async fn is_auth_valid(&self, registry_server: &str) -> bool {
        let cache = self.cache.read().await;
        cache
            .get(registry_server)
            .map(|auth| !auth.is_expired())
            .unwrap_or(false)
    }

    /// Cache successful authentication
    pub async fn cache_auth(&self, registry_server: String, ttl: Option<Duration>) {
        let mut cache = self.cache.write().await;
        let auth = CachedAuth::new(ttl.unwrap_or(self.default_ttl));
        
        debug!(
            registry = registry_server,
            ttl = ?auth.ttl,
            "Caching authentication"
        );
        
        cache.insert(registry_server, auth);
    }

    /// Invalidate cached authentication
    pub async fn invalidate(&self, registry_server: &str) {
        let mut cache = self.cache.write().await;
        if cache.remove(registry_server).is_some() {
            warn!(registry = registry_server, "Invalidated cached authentication");
        }
    }

    /// Clean expired entries
    pub async fn cleanup_expired(&self) {
        let mut cache = self.cache.write().await;
        cache.retain(|server, auth| {
            if auth.is_expired() {
                debug!(registry = server, "Removing expired authentication cache");
                false
            } else {
                true
            }
        });
    }
}

impl Default for RegistryAuthCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(30 * 60)) // 30 minutes default
    }
}

lazy_static::lazy_static! {
    pub static ref REGISTRY_AUTH_CACHE: Arc<RegistryAuthCache> = Arc::new(RegistryAuthCache::default());
}
