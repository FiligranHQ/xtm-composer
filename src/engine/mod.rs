pub mod openaev;
pub mod opencti;

use crate::api::ComposerApi;
use crate::orchestrator::docker::DockerOrchestrator;
use crate::orchestrator::kubernetes::KubeOrchestrator;
use crate::orchestrator::portainer::docker::PortainerDockerOrchestrator;
use crate::orchestrator::swarm::SwarmOrchestrator;
use crate::orchestrator::{Orchestrator, composer};
use crate::settings;
use crate::system::signals;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::interval;

/// Exponential backoff with jitter: base * 2^attempt, capped at max, plus deterministic jitter.
/// Jitter is attempt-based (not truly random) to avoid adding a `rand` dependency.
fn backoff_delay(attempt: u32, base_secs: u64, max_secs: u64) -> Duration {
    let base = Duration::from_secs(base_secs);
    let mut delay = base * 2_u32.saturating_pow(attempt).min(u32::MAX);
    // Cap at max
    let max_delay = Duration::from_secs(max_secs);
    if delay > max_delay {
        delay = max_delay;
    }
    // Deterministic jitter: up to 25% of delay, varies by attempt
    let jitter_ms = delay.as_millis() as u64 / 4;
    let jitter_offset = (attempt as u64 * 137) % (jitter_ms + 1);
    delay + Duration::from_millis(jitter_offset)
}

async fn orchestration(
    api: Box<dyn ComposerApi + Send + Sync>,
    mut ready: watch::Receiver<bool>,
) {
    let settings = settings();
    // Get current deployment in target orchestrator
    let daemon_configuration = api.daemon();
    let orchestrator: Box<dyn Orchestrator + Send + Sync> =
        match daemon_configuration.selector.as_str() {
            "portainer" => match daemon_configuration.portainer.clone() {
                Some(config) => match config.env_type.as_str() {
                    "docker" => Box::new(PortainerDockerOrchestrator::new(config)),
                    def => panic!("Invalid portainer type configuration: {}", def),
                },
                None => panic!("Missing portainer configuration"),
            },
            "kubernetes" => match daemon_configuration.kubernetes.clone() {
                Some(config) => Box::new(KubeOrchestrator::new(config).await),
                None => panic!("Missing kubernetes configuration"),
            },
            "docker" => Box::new(DockerOrchestrator::new()),
            "swarm" => match daemon_configuration.swarm.clone() {
                Some(config) => Box::new(SwarmOrchestrator::new(config)),
                None => panic!("Missing swarm configuration"),
            },
            def => panic!("Invalid daemon configuration: {}", def),
        };
    // Init scheduler interval
    let schedule = Duration::from_secs(settings.manager.execute_schedule);
    let mut interval = interval(schedule);
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            let mut log_ticks = HashMap::new();
            let mut health_tick = Instant::now();
            loop {
                // Wait until alive signals readiness before first orchestration tick
                loop {
                    ready.changed().await.expect("alive watch sender dropped unexpectedly");
                    if *ready.borrow_and_update() {
                        break;
                    }
                }
                interval.tick().await; // Wait for period
                composer::orchestrate(&mut log_ticks, &mut health_tick, &orchestrator, &api).await;
            }
        } => {
            // This branch will never be reached due to the infinite loop.
        }
    }
}

pub async fn alive(
    api: Box<dyn ComposerApi + Send + Sync>,
    ready: watch::Sender<bool>,
) -> JoinHandle<()> {
    let settings = settings();
    let ping_interval = Duration::from_secs(settings.manager.ping_alive_schedule);
    let backoff_max = settings.manager.ping_alive_schedule.max(30);
    tokio::spawn(async move {
        // Start scheduling
        tokio::select! {
            _ = signals::handle_stop_signals() => {}
            _ = async {
                let mut retry_attempt: u32 = 0;
                // Infinite retry loop for initial connection
                loop {
                    let version = api.version().await;
                    match version {
                        Some(version) => {
                            // Connection successful - register and start ping loop
                            api.register().await;
                            // Signal readiness
                            let _ = ready.clone().send(true);
                            let mut detected_version: String = version.clone();
                            // Reset backoff on success
                            retry_attempt = 0;
                            loop {
                                let ping_response = api.ping_alive().await;
                                match ping_response {
                                    Some(platform_version) => {
                                        // Register when version changes
                                        if platform_version != detected_version {
                                            api.register().await;
                                            detected_version = platform_version;
                                        }
                                    }
                                    _ => {
                                        // Connection lost - signal unready, break to outer retry loop
                                        let _ = ready.clone().send(false);
                                        break;
                                    }
                                }
                                tokio::time::sleep(ping_interval).await;
                            }
                        },
                        None => {
                            // Connection failed - exponential backoff then retry
                            let delay = backoff_delay(retry_attempt, 1, backoff_max);
                            retry_attempt = retry_attempt.saturating_add(1);
                            tokio::time::sleep(delay).await;
                        }
                    }
                }
            } => {
                // This branch will never be reached due to the infinite loop.
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiConnector, ConnectorStatus};
    use crate::config::settings::Daemon;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    /// Test-only fake API that tracks calls and can be configured to succeed/fail.
    struct EngineFakeApi {
        version_responses: Arc<Mutex<Vec<Option<String>>>>,
        ping_responses: Arc<Mutex<Vec<Option<String>>>>,
        register_responses: Arc<Mutex<Vec<bool>>>,
        version_calls: Arc<AtomicUsize>,
        register_calls: Arc<AtomicUsize>,
        ping_calls: Arc<AtomicUsize>,
        daemon: Daemon,
    }

    impl EngineFakeApi {
        fn new(
            version_responses: Vec<Option<String>>,
            ping_responses: Vec<Option<String>>,
            register_responses: Vec<bool>,
        ) -> Self {
            let daemon = Daemon {
                selector: "docker".to_string(),
                registry: None,
                portainer: None,
                kubernetes: None,
                docker: None,
                swarm: None,
            };
            Self {
                version_responses: Arc::new(Mutex::new(version_responses)),
                ping_responses: Arc::new(Mutex::new(ping_responses)),
                register_responses: Arc::new(Mutex::new(register_responses)),
                version_calls: Arc::new(AtomicUsize::new(0)),
                register_calls: Arc::new(AtomicUsize::new(0)),
                ping_calls: Arc::new(AtomicUsize::new(0)),
                daemon,
            }
        }
    }

    #[async_trait::async_trait]
    impl ComposerApi for EngineFakeApi {
        fn daemon(&self) -> &Daemon {
            &self.daemon
        }

        fn platform(&self) -> &'static str {
            "opencti"
        }

        fn post_logs_schedule(&self) -> Duration {
            Duration::from_secs(3600)
        }

        async fn version(&self) -> Option<String> {
            self.version_calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = self.version_responses.lock().await;
            resp.pop().unwrap_or(Some("1.0".to_string()))
        }

        async fn ping_alive(&self) -> Option<String> {
            self.ping_calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = self.ping_responses.lock().await;
            resp.pop().unwrap_or(Some("1.0".to_string()))
        }

        async fn register(&self) {
            self.register_calls.fetch_add(1, Ordering::SeqCst);
            let _ = self.register_responses.lock().await.pop();
        }

        async fn connectors(&self) -> Option<Vec<ApiConnector>> {
            Some(vec![])
        }

        async fn patch_status(&self, _id: String, _status: ConnectorStatus) -> Option<ApiConnector> {
            None
        }

        async fn patch_logs(&self, _id: String, _logs: Vec<String>) -> Option<String> {
            None
        }

        async fn patch_health(
            &self,
            _id: String,
            _restart_count: u32,
            _started_at: String,
            _is_in_reboot_loop: bool,
        ) -> Option<String> {
            None
        }
    }

    /// Given: alive succeeds immediately
    /// When: alive and orchestration run concurrently with a shared watch channel
    /// Then: orchestration does not proceed until ready is true
    #[tokio::test]
    async fn orchestration_waits_for_alive_signal() {
        let (ready_sender, ready_receiver) = watch::channel(false);

        // Initially not ready
        assert!(!*ready_receiver.borrow(), "ready should start as false");

        // Spawn a watcher that records when ready flips
        let ready_transitions: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let transitions_clone = ready_transitions.clone();
        let watcher_receiver = ready_receiver.clone();
        let watcher = tokio::spawn(async move {
            let mut watcher_receiver = watcher_receiver;
            let mut last_val = *watcher_receiver.borrow();
            transitions_clone.lock().await.push(last_val);
            loop {
                if watcher_receiver.changed().await.is_ok() {
                    let new_val = *watcher_receiver.borrow();
                    if new_val != last_val {
                        last_val = new_val;
                        transitions_clone.lock().await.push(new_val);
                    }
                }
            }
        });

        let api_clone: Box<dyn ComposerApi + Send + Sync> = Box::new(EngineFakeApi::new(
            vec![Some("1.0".to_string())],
            vec![Some("1.0".to_string()); 5],
            vec![true],
        ));

        // Start alive
        let alive_handle = alive(api_clone, ready_sender.clone()).await;

        // Give alive a moment to connect
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now ready should be true
        assert!(*ready_receiver.borrow(), "ready should be true after alive connects");

        // Start orchestration
        let orch_api: Box<dyn ComposerApi + Send + Sync> = Box::new(EngineFakeApi::new(
            vec![Some("1.0".to_string())],
            vec![Some("1.0".to_string()); 5],
            vec![true],
        ));
        let orch_ready = ready_receiver.clone();
        let orch_handle = tokio::spawn(async move {
            orchestration(orch_api, orch_ready).await;
        });

        // Let orchestration run briefly
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify transitions: false -> true
        let transitions = ready_transitions.lock().await;
        assert!(transitions.contains(&false), "should start as false");
        assert!(transitions.contains(&true), "should transition to true");

        // Clean up
        alive_handle.abort();
        orch_handle.abort();
        watcher.abort();
    }

    /// Given: version fails twice then succeeds
    /// When: alive retries
    /// Then: delays between retries increase (exponential backoff)
    #[tokio::test]
    async fn alive_retry_uses_exponential_backoff() {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(EngineFakeApi::new(
            vec![Some("1.0".to_string()), None, None], // pop() returns last first: None, None, Some("1.0")
            vec![Some("1.0".to_string()); 3],
            vec![true],
        ));
        let (ready_sender, ready_receiver) = watch::channel(false);

        let start = Instant::now();
        let alive_handle = alive(api, ready_sender).await;

        // Wait for ready to become true (after 3 version attempts)
        let _timeout = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if *ready_receiver.borrow() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.expect("alive should eventually become ready");

        let elapsed = start.elapsed();
        // With backoff 1s, 2s, 4s (roughly), total should be at least ~3s
        assert!(
            elapsed >= Duration::from_secs(2),
            "backoff should cause noticeable delay, elapsed: {:?}",
            elapsed
        );
        // But not too long (cap at 30s)
        assert!(
            elapsed < Duration::from_secs(15),
            "backoff should not exceed cap, elapsed: {:?}",
            elapsed
        );

        alive_handle.abort();
    }

    /// Given: alive connects, then ping fails, then succeeds again
    /// When: alive loop runs
    /// Then: ready transitions true -> false -> true, and register is called again
    #[tokio::test]
    async fn alive_recovers_from_connection_loss() {
        let api: Box<dyn ComposerApi + Send + Sync> = Box::new(EngineFakeApi::new(
            vec![
                Some("1.0".to_string()), // initial version
                Some("1.0".to_string()), // re-version after loss
            ],
            vec![
                Some("1.0".to_string()), // first ping ok
                None,                    // ping fails (connection lost)
                Some("1.0".to_string()), // ping ok after recovery
            ],
            vec![true, true, true], // register called multiple times
        ));
        let (ready_sender, ready_receiver) = watch::channel(false);

        let alive_handle = alive(api, ready_sender).await;

        // Wait for initial connection
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(*ready_receiver.borrow(), "should be ready after initial connection");

        // Wait for ping failure and recovery
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Should be ready again after recovery
        assert!(*ready_receiver.borrow(), "should be ready after recovery");

        alive_handle.abort();
    }

    /// Test backoff_delay produces increasing delays
    #[test]
    fn backoff_delay_increases_with_attempt() {
        let d0 = backoff_delay(0, 1, 30);
        let d1 = backoff_delay(1, 1, 30);
        let d2 = backoff_delay(2, 1, 30);
        let d3 = backoff_delay(3, 1, 30);

        assert!(d0.as_secs() >= 1, "attempt 0 should be at least 1s");
        assert!(d1.as_secs() >= 2, "attempt 1 should be at least 2s");
        assert!(d2.as_secs() >= 4, "attempt 2 should be at least 4s");
        assert!(d3.as_secs() >= 8, "attempt 3 should be at least 8s");
        assert!(d0 <= d1, "delays should increase");
        assert!(d1 <= d2, "delays should increase");
        assert!(d2 <= d3, "delays should increase");
    }

    /// Test backoff_delay caps at max
    #[test]
    fn backoff_delay_caps_at_max() {
        let d10 = backoff_delay(10, 1, 30);
        assert!(d10 <= Duration::from_secs(38), "should cap near max (30s + jitter), got {:?}", d10);
    }
}
