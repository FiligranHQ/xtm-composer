use tracing::error;
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::manager::ConnectorManager;
use crate::settings;

use cynic;
use crate::api::opencti::opencti as schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateConnectorManagerStatusVariables<'a> {
    pub input: UpdateConnectorManagerStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "UpdateConnectorManagerStatusVariables")]
pub struct UpdateConnectorManagerStatus {
    #[arguments(input: $input)]
    pub update_connector_manager_status: Option<ConnectorManager>,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateConnectorManagerStatusInput<'a> {
    pub id: &'a cynic::Id,
}

pub async fn ping_alive(api: &ApiOpenCTI) -> () {
    use cynic::MutationBuilder;

    let settings = settings();
    let vars = UpdateConnectorManagerStatusVariables {
        input: UpdateConnectorManagerStatusInput {
            id: &cynic::Id::new(&settings.manager.id),
        },
    };
    let mutation = UpdateConnectorManagerStatus::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    let _response = mutation_response.data.unwrap().update_connector_manager_status;
    let query_errors = mutation_response.errors.unwrap_or_default();
    if !query_errors.is_empty() {
        let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
        error!(error = errors.join(","), "Fail to ping api");
    }
}