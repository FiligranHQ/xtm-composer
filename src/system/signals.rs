use tracing::info;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal as unix_signal};

#[cfg(unix)]
pub async fn handle_stop_signals() -> Option<()> {
    let mut sigterm_stream = unix_signal(SignalKind::terminate()).ok()?;
    tokio::select! {
        _ = sigterm_stream.recv() => {
            info!("SIGTERM received.  Exiting gracefully.");
            Some(())
        }
        else => Some(())
    }
}

#[cfg(not(unix))]
pub async fn handle_stop_signals() -> Option<()> {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };
    tokio::select! {
        _ = ctrl_c => {
            info!("Ctrl+C received, exiting.");
            None
        }
        else => Some(())
    }
}
