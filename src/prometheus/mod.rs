use axum::{Router, routing::get};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::net::SocketAddr;
use std::sync::LazyLock;
use tracing::info;

// Registry initialization
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(|| Registry::new());

// region Metrics initialization
pub static MANAGED_CONNECTORS: LazyLock<IntGauge> = LazyLock::new(|| {
    let gauge = IntGauge::new(
        "xtm_managed_connectors",
        "Number of protected connectors managed by the composer",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("collector can be registered");
    gauge
});

pub static CONNECTOR_INITIALIZED: LazyLock<IntCounter> = LazyLock::new(|| {
    let counter = IntCounter::new(
        "xtm_connectors_initialized_total",
        "Number of connectors initialized",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(counter.clone()))
        .expect("collector can be registered");
    counter
});

pub static CONNECTOR_STARTED: LazyLock<IntCounter> = LazyLock::new(|| {
    let counter = IntCounter::new(
        "xtm_connectors_started_total",
        "Number of connectors started",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(counter.clone()))
        .expect("collector can be registered");
    counter
});

pub static CONNECTOR_STOPPED: LazyLock<IntCounter> = LazyLock::new(|| {
    let counter = IntCounter::new(
        "xtm_connectors_stopped_total",
        "Number of connectors stopped",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(counter.clone()))
        .expect("collector can be registered");
    counter
});

pub static CONNECTOR_UPDATED: LazyLock<IntCounter> = LazyLock::new(|| {
    let counter = IntCounter::new(
        "xtm_connectors_updated_total",
        "Number of connectors updated",
    )
    .expect("metric can be created");
    REGISTRY
        .register(Box::new(counter.clone()))
        .expect("collector can be registered");
    counter
});
// endregion

// Functions
async fn metrics_handler() -> String {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

pub async fn start_metrics_server(port: u16) {
    let app = Router::new().route("/metrics", get(metrics_handler));
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Prometheus server listening on {}/metrics", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
