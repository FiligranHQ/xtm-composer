pub mod api;
pub mod config;
pub mod engine;
pub mod orchestrator;
pub mod system;

use crate::config::settings::Settings;
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey};
use std::sync::OnceLock;
use std::fs;
use tracing::warn;

// Singleton settings for all application
pub fn settings() -> &'static Settings {
    static CONFIG: OnceLock<Settings> = OnceLock::new();
    CONFIG.get_or_init(|| Settings::new().unwrap())
}

// Singleton RSA private key for all application
pub fn private_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| load_and_verify_credentials_key())
}

// Load and verify RSA private key from configuration
pub fn load_and_verify_credentials_key() -> RsaPrivateKey {
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
    match RsaPrivateKey::from_pkcs8_pem(&key_content) {
        Ok(key) => {
            tracing::info!("Successfully loaded RSA private key (PKCS#8 format)");
            key
        },
        Err(e) => {
            panic!("Failed to decode RSA private key: {}", e);
        }
    }
}
