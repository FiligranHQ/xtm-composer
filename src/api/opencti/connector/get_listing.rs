use crate::api::ApiConnector;
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;
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

    let query = GetConnectors::build({});
    let get_connectors = api.query_fetch(query).await;
    match get_connectors {
        Ok(connectors_response) => {
            let query_errors = connectors_response.errors.unwrap_or_default();
            if !query_errors.is_empty() {
                let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
                error!(error = errors.join(","), "Fail to list connectors");
                None
            } else {
                let connectors = connectors_response
                    .data
                    .unwrap()
                    .connectors_for_managers
                    .unwrap()
                    .into_iter()
                    .map(|managed_connector| managed_connector.to_api_connector())
                    .collect();
                Some(connectors)
            }
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to fetch connectors");
            None
        }
    }
}
