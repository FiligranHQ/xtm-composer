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
#[cynic(graphql_type = "Query", variables = "GetConnectorsVariables")]
pub struct GetConnectors {
    #[arguments(managerId: $manager_id)]
    pub connectors_for_manager: Option<Vec<ManagedConnector>>,
}
// endregion

pub async fn list(api: &ApiOpenCTI) -> Option<Vec<ApiConnector>> {
    use cynic::QueryBuilder;

    let settings = crate::settings();
    let manager_id = settings.manager.id.clone();
    let vars = GetConnectorsVariables {
        manager_id: (&manager_id).into(),
    };
    let query = GetConnectors::build(vars);
    let get_connectors = api.query_fetch(query).await;
    match get_connectors {
        Ok(connectors) => {
            let connectors = connectors
                .data
                .unwrap()
                .connectors_for_manager
                .unwrap()
                .into_iter()
                .map(|managed_connector| managed_connector.to_api_connector())
                .collect();
            Some(connectors)
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to fetch connectors");
            None
        }
    }
}
