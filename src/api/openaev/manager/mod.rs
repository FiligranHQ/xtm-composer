pub mod get_version;
pub mod post_register;
pub mod ping_alive;

use serde::Deserialize;

/// The composer version reported to OpenAEV at registration and on each connectivity refresh.
pub const COMPOSER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
pub struct ConnectorManager {
    pub xtm_composer_id: String,
    pub xtm_composer_version: String,
}

/// The container image reference the composer reports (setting `manager.image`, environment
/// variable `MANAGER__IMAGE`), trimmed; `None` when unset or blank.
pub fn composer_image(configured: Option<&str>) -> Option<String> {
    configured
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_image_is_trimmed_and_blank_is_none() {
        assert_eq!(composer_image(None), None);
        assert_eq!(composer_image(Some("  ")), None);
        assert_eq!(
            composer_image(Some(" filigran/xtm-composer:3.261121.0 ")).as_deref(),
            Some("filigran/xtm-composer:3.261121.0")
        );
    }
}
