use crate::config::settings::Registry;
use crate::config::SecretString;
use crate::orchestrator::registry_resolver::RegistryResolver;
use std::sync::Arc;
use tokio::task::JoinSet;

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

#[tokio::test]
async fn test_concurrent_cache_access() {
    let registry = create_test_registry(Some("docker.io".to_string()), true);
    let resolver = Arc::new(RegistryResolver::new(Some(registry)));
    
    // Spawn 10 concurrent tasks trying to get credentials
    let mut tasks = JoinSet::new();
    for i in 0..10 {
        let resolver_clone = Arc::clone(&resolver);
        tasks.spawn(async move {
            let result = resolver_clone.get_docker_credentials();
            (i, result.is_ok())
        });
    }
    
    // All tasks should succeed
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    
    assert_eq!(results.len(), 10);
    assert!(results.iter().all(|(_, ok)| *ok));
}

#[tokio::test]
async fn test_cache_expiration_and_renewal() {
    let registry = Registry {
        server: Some("docker.io".to_string()),
        username: Some(SecretString::new("user".to_string())),
        password: Some(SecretString::new("pass".to_string())),
        email: None,
        auto_refresh_secret: false,
        refresh_threshold: 0.8,
    };
    
    let resolver = RegistryResolver::new(Some(registry));
    
    // First access - should cache
    let result1 = resolver.get_docker_credentials();
    assert!(result1.is_ok());
    
    // Wait for cache to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Second access - should refresh cache
    let result2 = resolver.get_docker_credentials();
    assert!(result2.is_ok());
}

#[tokio::test]
async fn test_concurrent_reads_performance() {
    let registry = create_test_registry(Some("docker.io".to_string()), true);
    let resolver = Arc::new(RegistryResolver::new(Some(registry)));
    
    // Prime the cache
    let _ = resolver.get_docker_credentials();
    
    // Measure concurrent read performance
    let start = std::time::Instant::now();
    
    let mut tasks = JoinSet::new();
    for _ in 0..100 {
        let resolver_clone = Arc::clone(&resolver);
        tasks.spawn(async move {
            resolver_clone.get_docker_credentials().is_ok()
        });
    }
    
    let mut success_count = 0;
    while let Some(result) = tasks.join_next().await {
        if result.unwrap() {
            success_count += 1;
        }
    }
    
    let duration = start.elapsed();
    
    assert_eq!(success_count, 100);
    // All 100 concurrent cache reads should complete quickly (well under 1 second)
    assert!(duration.as_millis() < 1000, "Concurrent reads took too long: {:?}", duration);
}

#[tokio::test]
async fn test_cache_double_check_pattern() {
    let registry = Registry {
        server: Some("docker.io".to_string()),
        username: Some(SecretString::new("user".to_string())),
        password: Some(SecretString::new("pass".to_string())),
        email: None,
        auto_refresh_secret: false,
        refresh_threshold: 0.8,
    };
    
    let resolver = Arc::new(RegistryResolver::new(Some(registry)));
    
    // Prime the cache
    let _ = resolver.get_docker_credentials();
    
    // Wait for cache to expire
    tokio::time::sleep(tokio::time::Duration::from_millis(2100)).await;
    
    // Spawn multiple tasks that will all try to update the expired cache
    let mut tasks = JoinSet::new();
    for _ in 0..5 {
        let resolver_clone = Arc::clone(&resolver);
        tasks.spawn(async move {
            resolver_clone.get_docker_credentials().is_ok()
        });
    }
    
    // All should succeed - the double-check pattern ensures only one actually updates
    let mut success_count = 0;
    while let Some(result) = tasks.join_next().await {
        if result.unwrap() {
            success_count += 1;
        }
    }
    
    assert_eq!(success_count, 5);
}

#[tokio::test]
async fn test_clone_shares_cache() {
    let registry = create_test_registry(Some("docker.io".to_string()), true);
    let resolver1 = Arc::new(RegistryResolver::new(Some(registry)));
    let resolver2 = resolver1.clone();
    
    // First resolver primes the cache
    let result1 = resolver1.get_docker_credentials();
    assert!(result1.is_ok());
    
    // Second resolver (clone) should use the same cache
    let result2 = resolver2.get_docker_credentials();
    assert!(result2.is_ok());
    
    // Both should return credentials
    assert!(result1.unwrap().is_some());
    assert!(result2.unwrap().is_some());
}

#[tokio::test]
async fn test_no_credentials_concurrent() {
    let registry = create_test_registry(Some("docker.io".to_string()), false);
    let resolver = Arc::new(RegistryResolver::new(Some(registry)));
    
    // Spawn concurrent tasks without auth
    let mut tasks = JoinSet::new();
    for _ in 0..10 {
        let resolver_clone = Arc::clone(&resolver);
        tasks.spawn(async move {
            resolver_clone.get_docker_credentials()
        });
    }
    
    // All should return Ok(None)
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let res = result.unwrap();
        results.push(res);
    }
    
    assert_eq!(results.len(), 10);
    assert!(results.iter().all(|r| r.as_ref().unwrap().is_none()));
}
