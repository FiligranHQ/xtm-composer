use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::error_handler::{extract_optional_field, handle_graphql_response};
use crate::api::opencti::manager::ConnectorManager;

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

    let settings = &crate::config::settings::SETTINGS;
    let vars = UpdateConnectorManagerStatusVariables {
        input: UpdateConnectorManagerStatusInput {
            id: &cynic::Id::new(&settings.manager.id),
        },
    };
    let mutation = UpdateConnectorManagerStatus::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            handle_graphql_response(
                response,
                "update_connector_manager_status",
                "OpenCTI backend does not support XTM composer manager ping. The composer will continue to run but won't be able to report its status to OpenCTI."
            ).and_then(|data| {
                extract_optional_field(
                    data.update_connector_manager_status,
                    "update_connector_manager_status",
                    "update_connector_manager_status"
                ).map(|manager| manager.about_version)
            })
        }
        Err(err) => {
            error!(error = err.to_string(), "Fail to ping api");
            None
        }
    }
}
