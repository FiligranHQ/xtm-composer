pub mod get_version;
pub mod post_ping;
pub mod post_register;

use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryFragment, Debug)]
pub struct ConnectorManager {
    pub id: cynic::Id,
    #[cynic(rename = "about_version")]
    pub about_version: String,
}
