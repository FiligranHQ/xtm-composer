use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::manager::ConnectorManager;
use crate::settings;
use tracing::error;

use crate::api::opencti::opencti as schema;
use cynic;

// region schema
#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateConnectorManagerStatusVariables<'a> {
    pub input: UpdateConnectorManagerStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    variables = "UpdateConnectorManagerStatusVariables"
)]
pub struct UpdateConnectorManagerStatus {
    #[arguments(input: $input)]
    pub update_connector_manager_status: Option<ConnectorManager>,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateConnectorManagerStatusInput<'a> {
    pub id: &'a cynic::Id,
}
// endregion

pub async fn ping(api: &ApiOpenCTI) -> Option<String> {
    use cynic::MutationBuilder;

    let settings = settings();
    let vars = UpdateConnectorManagerStatusVariables {
        input: UpdateConnectorManagerStatusInput {
            id: &cynic::Id::new(&settings.manager.id),
        },
    };
    let mutation = UpdateConnectorManagerStatus::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            let query_errors = response.errors.unwrap_or_default();
            if !query_errors.is_empty() {
                let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
                error!(error = errors.join(","), "Fail to ping api");
                None
            } else {
                let version = response
                    .data
                    .unwrap()
                    .update_connector_manager_status
                    .unwrap()
                    .about_version;
                Some(version)
            }
        }
        Err(err) => {
            error!(error = err.to_string(), "Fail to ping api");
            None
        }
    }
}
