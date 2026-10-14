use base64::{engine::general_purpose, Engine as _};
use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use aws_lc_rs::rsa::{OaepPrivateDecryptingKey, PrivateDecryptingKey, OAEP_SHA256_MGF1SHA256};
#[cfg(not(feature = "fips"))]
use aws_lc_rs::rsa::Pkcs1PrivateDecryptingKey;
use tracing::warn;

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
        _ => {
            warn!(version, "Encryption version not handled");
            return Ok(String::new());
        }
    };
    if aes_key_iv_decrypted_bytes.len() < 44 {
        return Err("Decrypted AES key and IV too short".into());
    }
    let aes_key_bytes = &aes_key_iv_decrypted_bytes[0..32];
    let aes_iv_bytes = &aes_key_iv_decrypted_bytes[32..44];
    let mut encrypted_value_bytes = encrypted_bytes[513..].to_vec();

    let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, aes_key_bytes)?);
    let nonce = Nonce::try_assume_unique_for_key(aes_iv_bytes)?;
    let plaintext_result = cipher.open_in_place(nonce, Aad::empty(), &mut encrypted_value_bytes);
    match plaintext_result {
        Ok(plaintext) => {
            let decoded_value = str::from_utf8(plaintext)?.to_string();
            Ok(decoded_value)
        },
        Err(e) => {
            warn!(error = e.to_string(), "Fail to decode value");
            Ok(String::from(""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rsa::{KeySize, OaepPublicEncryptingKey, Pkcs1PublicEncryptingKey};
    use std::sync::OnceLock;

    // Platforms encrypt the AES key with a 4096-bit RSA key (512-byte block)
    fn private_key() -> &'static PrivateDecryptingKey {
        static KEY: OnceLock<PrivateDecryptingKey> = OnceLock::new();
        KEY.get_or_init(|| PrivateDecryptingKey::generate(KeySize::Rsa4096).expect("RSA key generation"))
    }

    // Builds a payload the way the platforms do: version, RSA(key || iv), AES-256-GCM(value)
    fn encrypt(version: u8, value: &str) -> String {
        let mut aes_key_iv = [0u8; 44];
        aws_lc_rs::rand::fill(&mut aes_key_iv).unwrap();

        let public_key = private_key().public_key();
        let mut encrypted_key_iv = vec![0u8; 512];
        match version {
            1 => {
                Pkcs1PublicEncryptingKey::new(public_key).unwrap()
                    .encrypt(&aes_key_iv, &mut encrypted_key_iv).unwrap();
            }
            _ => {
                OaepPublicEncryptingKey::new(public_key).unwrap()
                    .encrypt(&OAEP_SHA256_MGF1SHA256, &aes_key_iv, &mut encrypted_key_iv, None).unwrap();
            }
        }

        let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &aes_key_iv[0..32]).unwrap());
        let nonce = Nonce::try_assume_unique_for_key(&aes_key_iv[32..44]).unwrap();
        let mut encrypted_value = value.as_bytes().to_vec();
        cipher.seal_in_place_append_tag(nonce, Aad::empty(), &mut encrypted_value).unwrap();

        let payload = [&[version][..], &encrypted_key_iv, &encrypted_value].concat();
        general_purpose::STANDARD.encode(payload)
    }

    #[cfg(not(feature = "fips"))]
    #[test]
    fn decrypts_version_1() {
        let value = parse_aes_encrypted_value(private_key(), encrypt(1, "secret")).unwrap();
        assert_eq!(value, "secret");
    }

    #[cfg(feature = "fips")]
    #[test]
    fn rejects_version_1_in_fips_mode() {
        let error = parse_aes_encrypted_value(private_key(), encrypt(1, "secret")).unwrap_err();
        assert_eq!(error.to_string(), "Encryption version 1 (RSA PKCS#1 v1.5) is not allowed in FIPS mode");
    }

    #[test]
    fn decrypts_version_2() {
        let value = parse_aes_encrypted_value(private_key(), encrypt(2, "secret")).unwrap();
        assert_eq!(value, "secret");
    }

    #[test]
    fn returns_empty_value_for_unknown_version() {
        let mut payload = general_purpose::STANDARD.decode(encrypt(2, "secret")).unwrap();
        payload[0] = 3;
        let value = parse_aes_encrypted_value(private_key(), general_purpose::STANDARD.encode(payload)).unwrap();
        assert_eq!(value, "");
    }

    #[test]
    fn returns_empty_value_for_tampered_ciphertext() {
        let mut payload = general_purpose::STANDARD.decode(encrypt(2, "secret")).unwrap();
        let last = payload.len() - 1;
        payload[last] ^= 0xff;
        let value = parse_aes_encrypted_value(private_key(), general_purpose::STANDARD.encode(payload)).unwrap();
        assert_eq!(value, "");
    }

    #[test]
    fn rejects_too_short_value() {
        let error = parse_aes_encrypted_value(private_key(), general_purpose::STANDARD.encode([2u8; 10])).unwrap_err();
        assert_eq!(error.to_string(), "Encrypted value too short");
    }
}
