use crate::api::ApiConnector;
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;
use crate::api::opencti::error_handler::{extract_optional_field, handle_graphql_response};
use tracing::error;

// region schema
use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryVariables, Debug)]
pub struct GetConnectorsVariables<'a> {
    pub manager_id: &'a cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query")]
pub struct GetConnectors {
    pub connectors_for_managers: Option<Vec<ManagedConnector>>,
}
// endregion

pub async fn list(api: &ApiOpenCTI) -> Option<Vec<ApiConnector>> {
    use cynic::QueryBuilder;

    let query = GetConnectors::build(());
    let get_connectors = api.query_fetch(query).await;
    match get_connectors {
        Ok(response) => {
            handle_graphql_response(
                response,
                "connectors_for_managers",
                "OpenCTI backend does not support XTM composer connector listing. The composer cannot manage connectors without backend support."
            ).and_then(|data| {
                extract_optional_field(
                    data.connectors_for_managers,
                    "connectors_for_managers",
                    "connectors_for_managers"
                ).map(|connectors| {
                    connectors
                        .into_iter()
                        .map(|managed_connector| managed_connector.to_api_connector(&api.private_key))
                        .collect()
                })
            })
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to fetch connectors");
            None
        }
    }
}
