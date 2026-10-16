use base64::{engine::general_purpose, Engine as _};
use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use aws_lc_rs::rsa::{OaepPrivateDecryptingKey, PrivateDecryptingKey, OAEP_SHA256_MGF1SHA256};
#[cfg(not(feature = "fips"))]
use aws_lc_rs::rsa::Pkcs1PrivateDecryptingKey;
use tracing::error;
use crate::api::ApiContractConfig;

/// Decrypts a value encrypted by the platform for this manager.
///
/// Every failure is reported as an `Err`: callers must never mistake a value
/// they could not read for an empty one, or they would overwrite working
/// connector secrets with blanks.
pub fn parse_aes_encrypted_value(
    private_key: &PrivateDecryptingKey,
    encrypted_value: String
) -> Result<String, Box<dyn std::error::Error>> {
    let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_value)?;

    if encrypted_bytes.len() < 513 {
        return Err("Encrypted value too short".into());
    }

    let version = *encrypted_bytes.get(0)
        .ok_or("Encrypted value is empty")?;

    let aes_key_iv_encrypted_bytes = &encrypted_bytes[1..=512];
    let mut aes_key_iv_buffer = vec![0u8; private_key.key_size_bytes()];
    let aes_key_iv_decrypted_bytes: &[u8] = match version {
        #[cfg(not(feature = "fips"))]
        1 => Pkcs1PrivateDecryptingKey::new(private_key.clone())?
            .decrypt(aes_key_iv_encrypted_bytes, &mut aes_key_iv_buffer)?,
        // RSA PKCS#1 v1.5 is not an approved key transport scheme in FIPS mode
        #[cfg(feature = "fips")]
        1 => return Err("Encryption version 1 (RSA PKCS#1 v1.5) is not allowed in FIPS mode".into()),
        2 => OaepPrivateDecryptingKey::new(private_key.clone())?
            .decrypt(&OAEP_SHA256_MGF1SHA256, aes_key_iv_encrypted_bytes, &mut aes_key_iv_buffer, None)?,
        _ => return Err(format!("Encryption version {} not handled", version).into()),
    };
    let aes_key_bytes = aes_key_iv_decrypted_bytes
        .get(0..32)
        .ok_or("Decrypted AES key is too short")?;
    let aes_iv_bytes = aes_key_iv_decrypted_bytes
        .get(32..44)
        .ok_or("Decrypted AES IV is too short")?;
    let mut encrypted_value_bytes = encrypted_bytes[513..].to_vec();

    let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, aes_key_bytes)?);
    let nonce = Nonce::try_assume_unique_for_key(aes_iv_bytes)?;
    let plaintext = cipher
        .open_in_place(nonce, Aad::empty(), &mut encrypted_value_bytes)
        .map_err(|e| format!("Fail to decrypt value: {}", e))?;
    Ok(str::from_utf8(plaintext)?.to_string())
}

/// Resolves one contract configuration entry, decrypting it when needed.
///
/// An encrypted entry without any value has nothing to decrypt and resolves to
/// an empty value, as the platform sent it. Any other decryption failure is
/// returned as an `Err` so the connector is not deployed with a blank secret.
pub fn resolve_contract_config(
    private_key: &PrivateDecryptingKey,
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
    private_key: &PrivateDecryptingKey,
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use aws_lc_rs::rsa::{KeySize, OaepPublicEncryptingKey, Pkcs1PublicEncryptingKey};
    use std::sync::OnceLock;

    /// 4096-bit key, as the platform wraps the AES key in a 512-byte block.
    pub(crate) fn test_private_key() -> &'static PrivateDecryptingKey {
        static KEY: OnceLock<PrivateDecryptingKey> = OnceLock::new();
        KEY.get_or_init(|| PrivateDecryptingKey::generate(KeySize::Rsa4096).unwrap())
    }

    /// Encrypts like the platform: version byte, RSA-wrapped AES key + IV,
    /// then the AES-256-GCM ciphertext (tag appended).
    pub(crate) fn encrypt_with_key_material(version: u8, key_iv: &[u8], plaintext: &str) -> String {
        let public_key = test_private_key().public_key();
        let mut wrapped = vec![0u8; 512];
        match version {
            1 => {
                Pkcs1PublicEncryptingKey::new(public_key).unwrap()
                    .encrypt(key_iv, &mut wrapped).unwrap();
            }
            _ => {
                OaepPublicEncryptingKey::new(public_key).unwrap()
                    .encrypt(&OAEP_SHA256_MGF1SHA256, key_iv, &mut wrapped, None).unwrap();
            }
        }
        let aes_key = [7u8; 32];
        let iv = [9u8; 12];
        let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &aes_key).unwrap());
        let mut ciphertext = plaintext.as_bytes().to_vec();
        cipher
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(iv), Aad::empty(), &mut ciphertext)
            .unwrap();
        let mut bytes = vec![version];
        bytes.extend(wrapped);
        bytes.extend(ciphertext);
        general_purpose::STANDARD.encode(bytes)
    }

    pub(crate) fn encrypt(version: u8, plaintext: &str) -> String {
        let mut key_iv = vec![7u8; 32];
        key_iv.extend([9u8; 12]);
        encrypt_with_key_material(version, &key_iv, plaintext)
    }

    fn tamper(encrypted: &str, index: usize) -> String {
        let mut bytes = general_purpose::STANDARD.decode(encrypted).unwrap();
        let index = index.min(bytes.len() - 1);
        bytes[index] ^= 0xff;
        general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn decrypts_version_2_values() {
        let decrypted = parse_aes_encrypted_value(test_private_key(), encrypt(2, "s3cr3t"));
        assert_eq!(decrypted.unwrap(), "s3cr3t");
    }

    #[cfg(not(feature = "fips"))]
    #[test]
    fn decrypts_version_1_values() {
        let decrypted = parse_aes_encrypted_value(test_private_key(), encrypt(1, "s3cr3t"));
        assert_eq!(decrypted.unwrap(), "s3cr3t");
    }

    #[cfg(feature = "fips")]
    #[test]
    fn rejects_version_1_values_in_fips_mode() {
        let error = parse_aes_encrypted_value(test_private_key(), encrypt(1, "s3cr3t")).unwrap_err();
        assert_eq!(error.to_string(), "Encryption version 1 (RSA PKCS#1 v1.5) is not allowed in FIPS mode");
    }

    #[test]
    fn rejects_too_short_value() {
        let error = parse_aes_encrypted_value(test_private_key(), general_purpose::STANDARD.encode([2u8; 10])).unwrap_err();
        assert_eq!(error.to_string(), "Encrypted value too short");
    }

    #[test]
    fn unknown_version_is_an_error_not_an_empty_value() {
        let mut bytes = general_purpose::STANDARD.decode(encrypt(2, "s3cr3t")).unwrap();
        bytes[0] = 3;
        let result = parse_aes_encrypted_value(test_private_key(), general_purpose::STANDARD.encode(bytes));
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn aes_failure_is_an_error_not_an_empty_value() {
        let result = parse_aes_encrypted_value(test_private_key(), tamper(&encrypt(2, "s3cr3t"), usize::MAX));
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn rsa_failure_is_an_error() {
        // Same failure as a value encrypted for another manager key.
        let result = parse_aes_encrypted_value(test_private_key(), tamper(&encrypt(2, "s3cr3t"), 1));
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn truncated_key_material_is_an_error_not_a_panic() {
        let result = parse_aes_encrypted_value(
            test_private_key(),
            encrypt_with_key_material(2, &[7u8; 10], "s3cr3t"),
        );
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn resolve_configuration_reports_undecryptable_keys_instead_of_blanking() {
        let (configuration, undecryptable_keys) = resolve_contract_configuration(
            test_private_key(),
            "connector-1",
            vec![
                ("PLAIN".to_string(), Some("https://example.com".to_string()), false),
                ("GOOD_SECRET".to_string(), Some(encrypt(2, "s3cr3t")), true),
                ("BAD_SECRET".to_string(), Some(tamper(&encrypt(2, "x"), usize::MAX)), true),
                ("UNSET_SECRET".to_string(), None, true),
                ("EMPTY_SECRET".to_string(), Some(String::new()), true),
            ],
        );

        assert_eq!(undecryptable_keys, vec!["BAD_SECRET".to_string()]);
        let values: Vec<(&str, &str, bool)> = configuration
            .iter()
            .map(|c| (c.key.as_str(), c.value.as_str(), c.is_sensitive))
            .collect();
        assert_eq!(
            values,
            vec![
                ("PLAIN", "https://example.com", false),
                ("GOOD_SECRET", "s3cr3t", true),
                ("UNSET_SECRET", "", true),
                ("EMPTY_SECRET", "", true),
            ],
            "the undecryptable value must not be blanked into the configuration"
        );
    }
}
