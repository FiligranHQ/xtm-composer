use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::error_handler::handle_graphql_response;
use tracing::error;

// region schema
use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateConnectorHealthVariables<'a> {
    pub input: HealthConnectorStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "UpdateConnectorHealthVariables")]
pub struct UpdateConnectorHealth {
    #[arguments(input: $input)]
    pub update_connector_health: cynic::Id,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(rename_all = "snake_case")]
pub struct HealthConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub restart_count: i32,
    pub started_at: String,
    pub is_in_reboot_loop: bool,
}
// endregion

pub async fn health(
    id: String,
    restart_count: u32,
    started_at: String,
    is_in_reboot_loop: bool,
    api: &ApiOpenCTI,
) -> Option<cynic::Id> {
    use cynic::MutationBuilder;

    let vars = UpdateConnectorHealthVariables {
        input: HealthConnectorStatusInput {
            id: &cynic::Id::new(id),
            restart_count: restart_count as i32,
            started_at,
            is_in_reboot_loop,
        },
    };
    let mutation = UpdateConnectorHealth::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            handle_graphql_response(
                response,
                "update_connector_health",
                "OpenCTI backend does not support XTM composer health updates. The connector will continue to run but health metrics won't be sent to OpenCTI."
            ).map(|data| data.update_connector_health)
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to push health metrics");
            None
        }
    }
}
