use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};
use std::fs;
use std::sync::LazyLock;
use tracing::{info, warn};

// Singleton RSA private key for all application
pub static PRIVATE_KEY: LazyLock<RsaPrivateKey> =
    LazyLock::new(|| load_and_verify_credentials_key());

// Load and verify RSA private key from configuration
fn load_and_verify_credentials_key() -> RsaPrivateKey {
    let setting = &crate::config::settings::SETTINGS;

    // Priority: file > environment variable
    let key_content = if let Some(filepath) = &setting.manager.credentials_key_filepath {
        // Warning if both are set
        if setting.manager.credentials_key.is_some() {
            warn!(
                "Both credentials_key and credentials_key_filepath are set. Using filepath (priority)."
            );
        }

        // Read key from file
        match fs::read_to_string(filepath) {
            Ok(content) => content,
            Err(e) => panic!("Failed to read credentials key file '{}': {}", filepath, e),
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
    if !trimmed_content.contains("BEGIN PRIVATE KEY")
        || !trimmed_content.contains("END PRIVATE KEY")
    {
        panic!(
            "Invalid private key format. Expected PKCS#8 PEM format with 'BEGIN PRIVATE KEY' and 'END PRIVATE KEY' markers."
        );
    }

    // Parse and validate RSA private key using PKCS#8 format
    match RsaPrivateKey::from_pkcs8_pem(&key_content) {
        Ok(key) => {
            info!("Successfully loaded RSA private key (PKCS#8 format)");
            key
        }
        Err(e) => {
            panic!("Failed to decode RSA private key: {}", e);
        }
    }
}
