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
use tokio::time::{MissedTickBehavior, interval};

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

async fn orchestration(api: Box<dyn ComposerApi + Send + Sync>, ready: watch::Receiver<bool>) {
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
    orchestration_loop(api, orchestrator, ready, schedule).await;
}

async fn orchestration_loop(
    api: Box<dyn ComposerApi + Send + Sync>,
    orchestrator: Box<dyn Orchestrator + Send + Sync>,
    mut ready: watch::Receiver<bool>,
    schedule: Duration,
) {
    let mut interval = interval(schedule);
    // After waiting for readiness, run once then a full period later (no catch-up burst)
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            let mut log_ticks = HashMap::new();
            let mut health_tick = Instant::now();
            loop {
                // Wait until alive signals readiness (returns immediately while ready);
                // stop cleanly if alive is gone
                if ready.wait_for(|is_ready| *is_ready).await.is_err() {
                    return;
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
    tokio::spawn(alive_loop(api, ready, ping_interval, backoff_max))
}

async fn alive_loop(
    api: Box<dyn ComposerApi + Send + Sync>,
    ready: watch::Sender<bool>,
    ping_interval: Duration,
    backoff_max: u64,
) {
    // Start scheduling
    tokio::select! {
        _ = signals::handle_stop_signals() => {}
        _ = async {
            let mut retry_attempt: u32 = 0;
            // Infinite retry loop for initial connection
            loop {
                // Connected only when the version answers and registration succeeds
                let version = match api.version().await {
                    Some(version) => api.register().await.then_some(version),
                    None => None,
                };
                match version {
                    Some(version) => {
                        // Connection and registration successful - start ping loop
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
                                        let _ = api.register().await;
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
                        // Connection or registration failed - exponential backoff then retry
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiConnector, ConnectorStatus};
    use crate::config::settings::Daemon;
    use crate::orchestrator::fake::FakeOrchestrator;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    /// Test-only fake API that tracks calls and can be configured to succeed/fail.
    struct EngineFakeApi {
        version_responses: Arc<Mutex<Vec<Option<String>>>>,
        ping_responses: Arc<Mutex<Vec<Option<String>>>>,
        register_responses: Arc<Mutex<Vec<bool>>>,
        version_calls: Arc<AtomicUsize>,
        version_instants: Arc<std::sync::Mutex<Vec<tokio::time::Instant>>>,
        register_calls: Arc<AtomicUsize>,
        ping_calls: Arc<AtomicUsize>,
        connectors_calls: Arc<AtomicUsize>,
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
                version_instants: Arc::new(std::sync::Mutex::new(Vec::new())),
                register_calls: Arc::new(AtomicUsize::new(0)),
                ping_calls: Arc::new(AtomicUsize::new(0)),
                connectors_calls: Arc::new(AtomicUsize::new(0)),
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
            self.version_instants
                .lock()
                .expect("mutex should not be poisoned")
                .push(tokio::time::Instant::now());
            let mut resp = self.version_responses.lock().await;
            resp.pop().unwrap_or(Some("1.0".to_string()))
        }

        async fn ping_alive(&self) -> Option<String> {
            self.ping_calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = self.ping_responses.lock().await;
            resp.pop().unwrap_or(Some("1.0".to_string()))
        }

        async fn register(&self) -> bool {
            self.register_calls.fetch_add(1, Ordering::SeqCst);
            // Yield so readiness observers see each transition before the next send
            tokio::task::yield_now().await;
            self.register_responses.lock().await.pop().unwrap_or(true)
        }

        async fn connectors(&self) -> Option<Vec<ApiConnector>> {
            self.connectors_calls.fetch_add(1, Ordering::SeqCst);
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

    fn fake_orchestrator() -> Box<dyn Orchestrator + Send + Sync> {
        Box::new(FakeOrchestrator::new(
            Vec::new(),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        ))
    }

    const SCHEDULE: Duration = Duration::from_secs(10);

    /// Fake API whose connector-list requests are counted, plus that counter.
    fn counting_api() -> (Box<dyn ComposerApi + Send + Sync>, Arc<AtomicUsize>) {
        let api = EngineFakeApi::new(vec![], vec![], vec![]);
        let connectors_calls = api.connectors_calls.clone();
        (Box::new(api), connectors_calls)
    }

    /// Given: the composer is connected and registered
    /// When: three orchestration periods elapse without any connectivity change
    /// Then: the connector list has been requested at least three times
    #[tokio::test(start_paused = true)]
    async fn orchestration_keeps_running_while_ready() {
        let (api, connectors_calls) = counting_api();
        let (ready_sender, ready_receiver) = watch::channel(false);
        // Readiness is signalled before orchestration starts running
        ready_sender.send(true).expect("receiver alive");
        let handle = tokio::spawn(orchestration_loop(
            api,
            fake_orchestrator(),
            ready_receiver,
            SCHEDULE,
        ));

        tokio::time::sleep(SCHEDULE * 3 + SCHEDULE / 2).await;
        let calls = connectors_calls.load(Ordering::SeqCst);
        handle.abort();

        assert!(
            calls >= 3,
            "orchestration should run every period while ready, got {calls} connector requests over 3 periods"
        );
    }

    /// Given: orchestration is waiting for the composer to become ready
    /// When: the connectivity monitor stops (readiness sender dropped)
    /// Then: orchestration stops without a crash
    #[tokio::test(start_paused = true)]
    async fn orchestration_stops_cleanly_when_alive_stops() {
        let (api, _connectors_calls) = counting_api();
        let (ready_sender, ready_receiver) = watch::channel(false);
        let handle = tokio::spawn(orchestration_loop(
            api,
            fake_orchestrator(),
            ready_receiver,
            SCHEDULE,
        ));
        // Let orchestration start waiting for readiness
        tokio::task::yield_now().await;

        drop(ready_sender);

        let outcome = tokio::time::timeout(SCHEDULE, handle)
            .await
            .expect("orchestration should finish once alive stops");
        if let Err(join_error) = outcome {
            assert!(
                !join_error.is_panic(),
                "orchestration crashed when alive stopped: {join_error}"
            );
        }
    }

    /// Given: the composer has not yet connected and registered
    /// When: several orchestration periods elapse
    /// Then: the composer has not requested the connector list
    #[tokio::test(start_paused = true)]
    async fn orchestration_does_not_fetch_connectors_before_ready() {
        let (api, connectors_calls) = counting_api();
        let (_ready_sender, ready_receiver) = watch::channel(false);
        let handle = tokio::spawn(orchestration_loop(
            api,
            fake_orchestrator(),
            ready_receiver,
            SCHEDULE,
        ));

        tokio::time::sleep(SCHEDULE * 5 + SCHEDULE / 2).await;
        let calls = connectors_calls.load(Ordering::SeqCst);
        handle.abort();

        assert_eq!(
            calls, 0,
            "connector list must not be requested before readiness"
        );
    }

    /// Given: orchestration waited for readiness for many orchestration periods
    /// When: the composer becomes ready
    /// Then: at most one orchestration runs immediately, the next one only after a full period
    #[tokio::test(start_paused = true)]
    async fn orchestration_does_not_burst_after_long_wait() {
        let (api, connectors_calls) = counting_api();
        let (ready_sender, ready_receiver) = watch::channel(false);
        let handle = tokio::spawn(orchestration_loop(
            api,
            fake_orchestrator(),
            ready_receiver,
            SCHEDULE,
        ));
        tokio::time::sleep(SCHEDULE * 10).await;

        ready_sender.send(true).expect("receiver alive");
        tokio::time::sleep(SCHEDULE / 10).await;
        let after_ready = connectors_calls.load(Ordering::SeqCst);
        tokio::time::sleep(SCHEDULE).await;
        let after_one_period = connectors_calls.load(Ordering::SeqCst);
        handle.abort();

        assert!(
            after_ready <= 1,
            "at most one orchestration should run right after readiness, got {after_ready}"
        );
        assert!(
            after_one_period - after_ready <= 1,
            "at most one orchestration per period, got {} in one period",
            after_one_period - after_ready
        );
    }

    const PING_INTERVAL: Duration = Duration::from_secs(10);
    const BACKOFF_MAX: u64 = 30;

    /// Spawn a watcher recording each observed readiness value with the
    /// number of registrations performed when it was observed.
    fn record_ready(
        mut ready: watch::Receiver<bool>,
        register_calls: Arc<AtomicUsize>,
    ) -> (Arc<std::sync::Mutex<Vec<(bool, usize)>>>, JoinHandle<()>) {
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = observed.clone();
        let watcher = tokio::spawn(async move {
            while ready.changed().await.is_ok() {
                let value = *ready.borrow_and_update();
                let mut sink = sink.lock().expect("mutex should not be poisoned");
                // Repeated sends of the same value are not transitions
                if sink.last().map(|(last, _)| *last) != Some(value) {
                    sink.push((value, register_calls.load(Ordering::SeqCst)));
                }
            }
        });
        (observed, watcher)
    }

    /// Given: the platform answers the version check
    /// And: the platform rejects registration twice and then accepts it
    /// When: the composer retries
    /// Then: the composer becomes ready only after the accepted registration
    #[tokio::test(start_paused = true)]
    async fn alive_ready_only_after_successful_registration() {
        // pop() returns last first: false, false, true
        let api = EngineFakeApi::new(vec![], vec![], vec![true, false, false]);
        let register_calls = api.register_calls.clone();
        let (ready_sender, ready_receiver) = watch::channel(false);
        let (observed, watcher) = record_ready(ready_receiver, register_calls.clone());
        let handle = tokio::spawn(alive_loop(
            Box::new(api),
            ready_sender,
            PING_INTERVAL,
            BACKOFF_MAX,
        ));

        tokio::time::sleep(Duration::from_secs(60)).await;
        handle.abort();
        watcher.abort();

        let observed = observed
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(
            observed,
            vec![(true, 3)],
            "ready must stay false until the third (accepted) registration, observed (ready, registrations): {observed:?}"
        );
        assert_eq!(register_calls.load(Ordering::SeqCst), 3);
    }

    /// Given: the platform is unreachable (version fails twice then succeeds)
    /// When: consecutive version checks fail
    /// Then: the waits between attempts follow the backoff schedule for attempts 0, 1
    #[tokio::test(start_paused = true)]
    async fn alive_retry_uses_exponential_backoff() {
        let api = EngineFakeApi::new(
            vec![Some("1.0".to_string()), None, None], // pop() returns last first: None, None, Some("1.0")
            vec![],
            vec![],
        );
        let version_instants = api.version_instants.clone();
        let (ready_sender, mut ready_receiver) = watch::channel(false);
        let handle = tokio::spawn(alive_loop(
            Box::new(api),
            ready_sender,
            PING_INTERVAL,
            BACKOFF_MAX,
        ));

        tokio::time::timeout(
            Duration::from_secs(60),
            ready_receiver.wait_for(|ready| *ready),
        )
        .await
        .expect("alive should become ready after the third version check")
        .expect("alive should still be running");
        handle.abort();

        let instants = version_instants
            .lock()
            .expect("mutex should not be poisoned")
            .clone();
        assert_eq!(instants.len(), 3, "expected three version checks");
        assert_eq!(
            instants[1] - instants[0],
            backoff_delay(0, 1, BACKOFF_MAX),
            "wait after attempt 0"
        );
        assert_eq!(
            instants[2] - instants[1],
            backoff_delay(1, 1, BACKOFF_MAX),
            "wait after attempt 1"
        );
    }

    /// Given: the composer is connected and registered
    /// When: a heartbeat fails and the platform then answers again
    /// Then: the composer registers again and becomes ready again
    #[tokio::test(start_paused = true)]
    async fn alive_reregisters_after_connection_loss() {
        let api = EngineFakeApi::new(
            vec![], // version always answers
            vec![
                None,                    // second ping fails (connection lost)
                Some("1.0".to_string()), // first ping ok (pop() returns last first)
            ],
            vec![], // registration always accepted
        );
        let register_calls = api.register_calls.clone();
        let (ready_sender, ready_receiver) = watch::channel(false);
        let (observed, watcher) = record_ready(ready_receiver, register_calls.clone());
        let handle = tokio::spawn(alive_loop(
            Box::new(api),
            ready_sender,
            PING_INTERVAL,
            BACKOFF_MAX,
        ));

        tokio::time::sleep(PING_INTERVAL * 6).await;
        handle.abort();
        watcher.abort();

        let transitions: Vec<bool> = observed
            .lock()
            .expect("mutex should not be poisoned")
            .iter()
            .map(|(ready, _)| *ready)
            .collect();
        assert_eq!(
            transitions,
            vec![true, false, true],
            "ready should go true -> false -> true"
        );
        assert_eq!(
            register_calls.load(Ordering::SeqCst),
            2,
            "the composer should register again after the connection is restored"
        );
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
