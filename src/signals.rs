use log::info;
use tokio::signal;

#[cfg(unix)]
use tokio::signal::unix::{signal as unix_signal, SignalKind};

#[cfg(unix)]
pub async fn handle_stop_signals() -> Option<()> {
    // SIGKILL (cannot be caught, but we can try to log it)
    let mut sigkill_stream = unix_signal(SignalKind::kill()).ok()?;

    // SIGTERM (graceful shutdown)
    let mut sigterm_stream = unix_signal(SignalKind::terminate()).ok()?;

    tokio::select! {
        _ = sigkill_stream.recv() => {
            println!("SIGKILL received.  Forced shutdown.");
            None // Indicate termination
        }
        _ = sigterm_stream.recv() => {
            println!("SIGTERM received.  Exiting gracefully.");
            // Perform cleanup here...
            Some(()) // Indicate graceful exit
        }
        else => Some(()) // Handle the case when both futures are not ready
    }
}

#[cfg(not(unix))]
pub async fn handle_stop_signals() -> Option<()> {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    tokio::select! {
        _ = ctrl_c => {
            info!("Ctrl+C received, exiting.");
            None
        }
        else => Some(()) // Handle the case when both futures are not ready
    }
}