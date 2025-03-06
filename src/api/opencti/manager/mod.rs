pub mod post_register;
pub mod post_ping;

use cynic;
use crate::api::opencti::opencti as schema;

#[derive(cynic::QueryFragment, Debug)]
pub struct ConnectorManager {
    pub id: cynic::Id,
}