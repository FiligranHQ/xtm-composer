use serde::{Deserialize, Serialize, Serializer};
use std::fmt;

/// A wrapper around String that redacts its content in Debug output
/// to prevent accidental secret leakage in logs, panics, or error messages.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as the redacted string for safety
        serializer.serialize_str("***REDACTED***")
    }
}

impl SecretString {
    /// Create a new SecretString
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Get the inner string value
    /// ⚠️ Use with caution - only when actually needed for authentication
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Convert to owned String (consumes self)
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "***REDACTED***")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "***REDACTED***")
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(SecretString)
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        SecretString(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_string_debug_redacted() {
        let secret = SecretString::new("super_secret_password".to_string());
        let debug_output = format!("{:?}", secret);
        assert_eq!(debug_output, "***REDACTED***");
        assert!(!debug_output.contains("super_secret"));
    }

    #[test]
    fn test_secret_string_display_redacted() {
        let secret = SecretString::new("super_secret_password".to_string());
        let display_output = format!("{}", secret);
        assert_eq!(display_output, "***REDACTED***");
    }

    #[test]
    fn test_secret_string_expose() {
        let secret = SecretString::new("my_password".to_string());
        assert_eq!(secret.expose_secret(), "my_password");
    }

    #[test]
    fn test_secret_string_deserialization() {
        use serde_json;
        let json = r#""my_secret""#;
        let secret: SecretString = serde_json::from_str(json).unwrap();
        assert_eq!(secret.expose_secret(), "my_secret");
    }
}
