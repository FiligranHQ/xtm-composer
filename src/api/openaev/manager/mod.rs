pub mod get_version;
pub mod post_register;
pub mod ping_alive;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ConnectorManager {
    pub xtm_composer_id: cynic::Id,
    pub xtm_composer_version: String,
}