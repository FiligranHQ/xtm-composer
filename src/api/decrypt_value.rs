use base64::{engine::general_purpose, Engine as _};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce
};
use rsa::{Oaep, Pkcs1v15Encrypt, RsaPrivateKey};
use tracing::warn;
use sha2::Sha256;

pub fn parse_aes_encrypted_value(
    private_key: &RsaPrivateKey,
    encrypted_value: String
) -> Result<String, Box<dyn std::error::Error>> {
    let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_value)?;

    let version = *encrypted_bytes.get(0)
        .ok_or("Encrypted value is empty")?;

    if encrypted_bytes.len() < 513 {
        return Err("Encrypted value too short".into());
    }

    let aes_key_iv_encrypted_bytes = &encrypted_bytes[1..=512];
    let aes_key_iv_decrypted_bytes = match version {
        1 => private_key.decrypt(Pkcs1v15Encrypt, aes_key_iv_encrypted_bytes)?,
        2 => private_key.decrypt(Oaep::new::<Sha256>(), aes_key_iv_encrypted_bytes)?,
        _ => {
            warn!(version, "Encryption version not handled");
            return Ok(String::new());
        }
    };
    let aes_key_bytes = &aes_key_iv_decrypted_bytes[0..32];
    let aes_iv_bytes = &aes_key_iv_decrypted_bytes[32..44];
    let encrypted_value_bytes = &encrypted_bytes[513..];

    let cipher = Aes256Gcm::new_from_slice(&aes_key_bytes)?;
    let nonce = Nonce::from_slice(&aes_iv_bytes);
    let plaintext_result = cipher.decrypt(&nonce, encrypted_value_bytes.as_ref());
    match plaintext_result {
        Ok(plaintext) => {
            let decoded_value = str::from_utf8(&plaintext)?.to_string();
            Ok(decoded_value)
        },
        Err(e) => {
            warn!(error = e.to_string(), "Fail to decode value");
            Ok(String::from(""))
        }
    }
}