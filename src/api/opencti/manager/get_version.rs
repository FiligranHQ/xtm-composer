use crate::api::opencti::error_handler::{extract_optional_field, handle_graphql_response};
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

    let query = GetVersion::build(());
    let get_version = api.query_fetch(query).await;
    match get_version {
        Ok(response) => {
            handle_graphql_response(
                response,
                "about",
                "OpenCTI backend does not support version query. This may indicate the backend doesn't support XTM composer."
            ).and_then(|data| {
                extract_optional_field(
                    data.about,
                    "about",
                    "about"
                ).map(|about| about.version)
            })
        }
        Err(e) => {
            error!(
                error = e.to_string(),
                "Fail to fetch version, check your configuration"
            );
            None
        }
    }
}
