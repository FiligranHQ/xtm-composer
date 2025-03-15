use tracing::error;

// region schema
use crate::api::opencti::{ApiOpenCTI, opencti as schema};
use cynic;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query")]
pub struct GetVersion {
    pub about: Option<AppInfo>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AppInfo {
    pub version: String,
}
// endregion

pub async fn version(api: &ApiOpenCTI) -> Option<String> {
    use cynic::QueryBuilder;

    let query = GetVersion::build({});
    let get_version = api.query_fetch(query).await;
    match get_version {
        Ok(version_response) => Some(version_response.data.unwrap().about.unwrap().version),
        Err(e) => {
            error!(
                error = e.to_string(),
                "Fail to fetch version, check your configuration"
            );
            None
        }
    }
}
