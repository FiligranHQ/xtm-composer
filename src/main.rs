mod api;
mod config;
mod engine;
mod orchestrator;
mod system;

use crate::config::settings::{Logger, Settings};
use crate::engine::openaev::{openaev_alive, openaev_orchestration};
use crate::engine::opencti::{opencti_alive, opencti_orchestration};
use futures::future::join_all;
use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;
use std::{env, fs};
use tokio::task::JoinHandle;
use tracing::{Level, Subscriber, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::Layer as _;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, layer::SubscriberExt};
use aws_lc_rs::rsa::PrivateDecryptingKey;
use rustls::crypto::CryptoProvider;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_DIRECTORY_LOG: &str = "logs";
const BASE_DIRECTORY_SIZE: usize = 5;
const PREFIX_LOG_NAME: &str = "xtm-composer.log";

// Singleton settings for all application
fn settings() -> &'static Settings {
    static CONFIG: OnceLock<Settings> = OnceLock::new();
    CONFIG.get_or_init(|| Settings::new().unwrap())
}

// Singleton RSA private key for all application
pub fn private_key() -> &'static PrivateDecryptingKey {
    static KEY: OnceLock<PrivateDecryptingKey> = OnceLock::new();
    KEY.get_or_init(|| load_and_verify_credentials_key())
}

// Global init logger
fn init_logger() -> WorkerGuard {
    let setting = Settings::new().unwrap();
    let logger_config = &setting.manager.logger;

    // Validate log level
    let log_level = match Level::from_str(&logger_config.level) {
        Ok(level) => level,
        Err(_) => panic!(
            "Invalid log level: '{}'. Valid values are: trace, debug, info, warn, error",
            logger_config.level
        )
    };

    // Validate log format
    if logger_config.format != "json" && logger_config.format != "pretty" {
        panic!(
            "Invalid log format: '{}'. Valid values are: json, pretty",
            logger_config.format
        );
    }

    let current_exe_patch = env::current_exe().unwrap();
    let parent_path = current_exe_patch.parent().unwrap();
    let (subscriber, guard) = build_subscriber(
        logger_config,
        log_level,
        &parent_path.join(BASE_DIRECTORY_LOG),
        std::io::stdout,
    );
    subscriber.init();
    guard
}

// Build the logging subscriber: `console` gates stdout, `directory` gates the log file.
// The returned guard flushes the non-blocking file writer and must outlive all logging.
fn build_subscriber<W>(
    logger_config: &Logger,
    log_level: Level,
    log_path: &Path,
    console_writer: W,
) -> (impl Subscriber + Send + Sync, WorkerGuard)
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let condition = RollingConditionBasic::new().daily();
    fs::create_dir(log_path).unwrap_or_default();
    let log_file = log_path.join(PREFIX_LOG_NAME);
    let file_appender =
        BasicRollingFileAppender::new(log_file, condition, BASE_DIRECTORY_SIZE).unwrap();
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let console_writer = console_writer.with_max_level(log_level);
    let file_layer = logger_config.directory.then(|| {
        Layer::new()
            .with_writer(file_writer.with_max_level(log_level))
            .json()
    });
    let console_layer = logger_config.console.then(|| {
        if logger_config.format == "json" {
            Layer::new().with_writer(console_writer).json().boxed()
        } else {
            Layer::new().with_writer(console_writer).pretty().boxed()
        }
    });
    let subscriber = Registry::default().with(console_layer).with(file_layer);
    (subscriber, guard)
}

// Load and verify RSA private key from configuration
pub fn load_and_verify_credentials_key() -> PrivateDecryptingKey {
    let setting = settings();
    
    // Priority: file > environment variable
    let key_content = if let Some(filepath) = &setting.manager.credentials_key_filepath {
        // Warning if both are set
        if setting.manager.credentials_key.is_some() {
            warn!("Both credentials_key and credentials_key_filepath are set. Using filepath (priority).");
        }
        
        // Read key from file
        match fs::read_to_string(filepath) {
            Ok(content) => content,
            Err(e) => panic!("Failed to read credentials key file '{}': {}", filepath, e)
        }
    } else if let Some(key) = &setting.manager.credentials_key {
        // Use environment variable or config value
        key.clone()
    } else {
        panic!(
            "No credentials key provided! Set either 'manager.credentials_key' or 'manager.credentials_key_filepath' in configuration."
        );
    };
    
    // Validate key format (trim to handle trailing whitespace)
    // Check for presence of RSA PRIVATE KEY markers for PKCS#8 format
    let trimmed_content = key_content.trim();
    if !trimmed_content.contains("BEGIN PRIVATE KEY") || !trimmed_content.contains("END PRIVATE KEY") {
        panic!("Invalid private key format. Expected PKCS#8 PEM format with 'BEGIN PRIVATE KEY' and 'END PRIVATE KEY' markers.");
    }
    
    // Parse and validate RSA private key using PKCS#8 format
    match api::rsa_key::private_key_from_pkcs8_pem(&key_content) {
        Ok(key) => {
            info!("Successfully loaded RSA private key (PKCS#8 format)");
            key
        },
        Err(e) => {
            panic!("Failed to decode RSA private key: {}", e);
        }
    }
}

fn opencti_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.opencti.enable {
        // Initialize private key singleton
        let _ = private_key();
        let opencti_alive = opencti_alive();
        orchestrations.push(opencti_alive);
        let opencti_orchestration = opencti_orchestration();
        orchestrations.push(opencti_orchestration);
    } else {
        info!("OpenCTI connectors orchestration disabled");
    }
}

// Init openaev
fn openaev_orchestrate(orchestrations: &mut Vec<JoinHandle<()>>) {
    let setting = settings();
    if setting.openaev.enable {
        let openaev_alive = openaev_alive();
        orchestrations.push(openaev_alive);
        let openaev_orchestration = openaev_orchestration();
        orchestrations.push(openaev_orchestration);
    } else {
        info!("OpenAEV connectors orchestration disabled");
    }
}

// Install the process-wide rustls CryptoProvider used by every TLS client
#[cfg(not(feature = "fips"))]
fn install_crypto_provider() {
    // Ignore error if a provider was already installed by another dependency.
    let _ = CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider());
}

// In FIPS mode, refuse to start unless TLS relies on the FIPS validated module
#[cfg(feature = "fips")]
fn install_crypto_provider() {
    if let Err(e) = aws_lc_rs::try_fips_mode() {
        panic!("AWS-LC is not running in FIPS mode: {}", e);
    }
    // A provider already installed by another dependency is kept, then checked below.
    let _ = CryptoProvider::install_default(rustls::crypto::default_fips_provider());
    let provider = CryptoProvider::get_default().expect("No rustls CryptoProvider installed");
    if !provider.fips() {
        panic!("The installed rustls CryptoProvider is not FIPS compliant");
    }
}

#[tokio::main]
async fn main() {
    // Install the rustls CryptoProvider before any TLS client is created.
    // Required since reqwest 0.13 switched from native-tls to rustls.
    install_crypto_provider();
    // Initialize the global logging system
    // Keep the guard alive for the whole run, otherwise the file writer thread stops
    let _log_guard = init_logger();
    // Log the start
    let env = Settings::mode();
    let fips = cfg!(feature = "fips");
    let image_tag_suffix = settings().manager.image_tag_suffix.as_deref().unwrap_or_default();
    info!(version = VERSION, env, fips, image_tag_suffix, "Starting XTM composer");
    // Start orchestration threads
    let mut orchestrations = Vec::new();
    opencti_orchestrate(&mut orchestrations);
    openaev_orchestrate(&mut orchestrations);
    // Wait for threads to terminate
    join_all(orchestrations).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_a_crypto_provider_matching_the_fips_feature() {
        install_crypto_provider();
        let provider = CryptoProvider::get_default().expect("No rustls CryptoProvider installed");
        assert_eq!(provider.fips(), cfg!(feature = "fips"));
    }

    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    // Returns (console output, file output) after logging one line
    fn log_once(name: &str, directory: bool, console: bool) -> (String, String) {
        let dir = env::temp_dir().join(format!("xtm-composer-logger-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config = Logger {
            level: "info".to_string(),
            format: "json".to_string(),
            directory,
            console,
        };
        let stdout = SharedBuf::default();
        let (subscriber, guard) = build_subscriber(&config, Level::INFO, &dir, stdout.clone());
        tracing::subscriber::with_default(subscriber, || info!("hello logger"));
        drop(guard); // flushes the non-blocking file writer
        let file = fs::read_to_string(dir.join(PREFIX_LOG_NAME)).unwrap_or_default();
        let console_out = String::from_utf8(stdout.0.lock().unwrap().clone()).unwrap();
        let _ = fs::remove_dir_all(&dir);
        (console_out, file)
    }

    #[test]
    fn console_only_logs_to_stdout_and_not_to_the_file() {
        let (console, file) = log_once("console-only", false, true);
        assert!(console.contains("hello logger"));
        assert!(file.is_empty());
    }

    #[test]
    fn directory_only_logs_to_the_file_and_not_to_stdout() {
        let (console, file) = log_once("directory-only", true, false);
        assert!(console.is_empty());
        assert!(file.contains("hello logger"));
    }

    #[test]
    fn both_switches_log_to_stdout_and_the_file() {
        let (console, file) = log_once("both", true, true);
        assert!(console.contains("hello logger"));
        assert!(file.contains("hello logger"));
    }

    #[test]
    fn no_switch_logs_nowhere() {
        let (console, file) = log_once("none", false, false);
        assert!(console.is_empty());
        assert!(file.is_empty());
    }
}
