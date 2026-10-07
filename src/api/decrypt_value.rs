use base64::{engine::general_purpose, Engine as _};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce
};
use rsa::{Oaep, Pkcs1v15Encrypt, RsaPrivateKey};
use sha2::Sha256;
use tracing::error;
use crate::api::ApiContractConfig;

/// Decrypts a value encrypted by the platform for this manager.
///
/// Every failure is reported as an `Err`: callers must never mistake a value
/// they could not read for an empty one, or they would overwrite working
/// connector secrets with blanks.
pub fn parse_aes_encrypted_value(
    private_key: &RsaPrivateKey,
    encrypted_value: String
) -> Result<String, Box<dyn std::error::Error>> {
    let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_value)?;

    if encrypted_bytes.len() < 513 {
        return Err("Encrypted value too short".into());
    }

    let version = *encrypted_bytes.get(0)
        .ok_or("Encrypted value is empty")?;

    let aes_key_iv_encrypted_bytes = &encrypted_bytes[1..=512];
    let aes_key_iv_decrypted_bytes = match version {
        1 => private_key.decrypt(Pkcs1v15Encrypt, aes_key_iv_encrypted_bytes)?,
        2 => private_key.decrypt(Oaep::new::<Sha256>(), aes_key_iv_encrypted_bytes)?,
        _ => return Err(format!("Encryption version {} not handled", version).into()),
    };
    let aes_key_bytes = aes_key_iv_decrypted_bytes
        .get(0..32)
        .ok_or("Decrypted AES key is too short")?;
    let aes_iv_bytes = aes_key_iv_decrypted_bytes
        .get(32..44)
        .ok_or("Decrypted AES IV is too short")?;
    let encrypted_value_bytes = &encrypted_bytes[513..];

    let cipher = Aes256Gcm::new_from_slice(aes_key_bytes)?;
    let nonce = Nonce::from_slice(aes_iv_bytes);
    let plaintext = cipher
        .decrypt(nonce, encrypted_value_bytes)
        .map_err(|e| format!("Fail to decrypt value: {}", e))?;
    Ok(str::from_utf8(&plaintext)?.to_string())
}

/// Resolves one contract configuration entry, decrypting it when needed.
///
/// An encrypted entry without any value has nothing to decrypt and resolves to
/// an empty value, as the platform sent it. Any other decryption failure is
/// returned as an `Err` so the connector is not deployed with a blank secret.
pub fn resolve_contract_config(
    private_key: &RsaPrivateKey,
    key: String,
    value: Option<String>,
    is_encrypted: bool,
) -> Result<ApiContractConfig, Box<dyn std::error::Error>> {
    let value = value.unwrap_or_default();
    let value = if is_encrypted && !value.is_empty() {
        parse_aes_encrypted_value(private_key, value)?
    } else {
        value
    };
    Ok(ApiContractConfig {
        key,
        value,
        is_sensitive: is_encrypted,
    })
}

/// Resolves a connector's whole contract configuration from
/// `(key, value, is_encrypted)` entries.
///
/// Returns the resolved entries and the keys that could not be decrypted.
/// Undecryptable entries are left out rather than blanked: the caller must
/// treat the configuration as incomplete and not deploy it.
pub fn resolve_contract_configuration(
    private_key: &RsaPrivateKey,
    connector_id: &str,
    entries: impl IntoIterator<Item = (String, Option<String>, bool)>,
) -> (Vec<ApiContractConfig>, Vec<String>) {
    let mut configuration = Vec::new();
    let mut undecryptable_keys = Vec::new();
    for (key, value, is_encrypted) in entries {
        match resolve_contract_config(private_key, key.clone(), value, is_encrypted) {
            Ok(config) => configuration.push(config),
            Err(err) => {
                error!(
                    connector_id = connector_id,
                    key = key,
                    error = %err,
                    "Fail to decrypt connector configuration value"
                );
                undecryptable_keys.push(key);
            }
        }
    }
    (configuration, undecryptable_keys)
}
