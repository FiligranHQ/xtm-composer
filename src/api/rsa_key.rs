use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rsa::{PrivateDecryptingKey, PublicKey};
use pem::{EncodeConfig, LineEnding, Pem};
use std::error::Error;

const PKCS8_PRIVATE_KEY_LABEL: &str = "PRIVATE KEY";
const PKCS1_PUBLIC_KEY_LABEL: &str = "RSA PUBLIC KEY";
const SPKI_PUBLIC_KEY_LABEL: &str = "PUBLIC KEY";

// Parse an RSA private key from a PKCS#8 PEM document
pub fn private_key_from_pkcs8_pem(content: &str) -> Result<PrivateDecryptingKey, Box<dyn Error>> {
    let document = pem::parse(content)?;
    if document.tag() != PKCS8_PRIVATE_KEY_LABEL {
        return Err(format!(
            "Unexpected PEM label '{}', expected '{}'",
            document.tag(),
            PKCS8_PRIVATE_KEY_LABEL
        )
        .into());
    }
    Ok(PrivateDecryptingKey::from_pkcs8(document.contents())?)
}

// Public key as a PKCS#1 PEM document (RSAPublicKey), expected by OpenCTI
pub fn public_key_pkcs1_pem(private_key: &PrivateDecryptingKey) -> Result<String, Box<dyn Error>> {
    let spki = private_key.public_key().as_der()?;
    let public_key = PublicKey::from_der(spki.as_ref())?;
    Ok(encode_pem(PKCS1_PUBLIC_KEY_LABEL, public_key.as_ref()))
}

// Public key as a SubjectPublicKeyInfo PEM document, expected by OpenAEV
pub fn public_key_spki_pem(private_key: &PrivateDecryptingKey) -> Result<String, Box<dyn Error>> {
    let spki = private_key.public_key().as_der()?;
    Ok(encode_pem(SPKI_PUBLIC_KEY_LABEL, spki.as_ref()))
}

fn encode_pem(label: &str, der: &[u8]) -> String {
    let config = EncodeConfig::new().set_line_ending(LineEnding::LF);
    pem::encode_config(&Pem::new(label, der), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::encoding::Pkcs8V1Der;
    use aws_lc_rs::rsa::KeySize;

    fn private_key_pem(private_key: &PrivateDecryptingKey) -> String {
        let pkcs8: Pkcs8V1Der = private_key.as_der().unwrap();
        encode_pem(PKCS8_PRIVATE_KEY_LABEL, pkcs8.as_ref())
    }

    #[test]
    fn loads_pkcs8_pem_private_key() {
        let private_key = PrivateDecryptingKey::generate(KeySize::Rsa2048).unwrap();
        let loaded = private_key_from_pkcs8_pem(&private_key_pem(&private_key)).unwrap();
        assert_eq!(
            loaded.public_key().as_der().unwrap().as_ref(),
            private_key.public_key().as_der().unwrap().as_ref()
        );
    }

    #[test]
    fn rejects_non_pkcs8_pem_label() {
        let document = encode_pem("RSA PRIVATE KEY", b"not a key");
        let error = private_key_from_pkcs8_pem(&document).unwrap_err();
        assert_eq!(error.to_string(), "Unexpected PEM label 'RSA PRIVATE KEY', expected 'PRIVATE KEY'");
    }

    #[test]
    fn rejects_invalid_pkcs8_content() {
        let document = encode_pem(PKCS8_PRIVATE_KEY_LABEL, b"not a key");
        assert!(private_key_from_pkcs8_pem(&document).is_err());
    }

    #[test]
    fn exports_public_key_as_pkcs1_and_spki_pem() {
        let private_key = PrivateDecryptingKey::generate(KeySize::Rsa2048).unwrap();
        let spki = private_key.public_key().as_der().unwrap();

        let pkcs1_document = pem::parse(public_key_pkcs1_pem(&private_key).unwrap()).unwrap();
        assert_eq!(pkcs1_document.tag(), PKCS1_PUBLIC_KEY_LABEL);
        let pkcs1_key = PublicKey::from_der(pkcs1_document.contents()).unwrap();
        assert_eq!(pkcs1_key.as_der().unwrap().as_ref(), spki.as_ref());

        let spki_document = pem::parse(public_key_spki_pem(&private_key).unwrap()).unwrap();
        assert_eq!(spki_document.tag(), SPKI_PUBLIC_KEY_LABEL);
        assert_eq!(spki_document.contents(), spki.as_ref());
    }
}
