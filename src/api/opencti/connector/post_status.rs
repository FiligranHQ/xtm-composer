use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;
use crate::api::opencti::error_handler::{extract_optional_field, handle_graphql_response};
use crate::api::{ApiConnector, ConnectorStatus};

use crate::api::opencti::opencti as schema;
use cynic;
use tracing::error;

// region schema
#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateConnectorCurrentStatusVariables<'a> {
    pub input: CurrentConnectorStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "UpdateConnectorCurrentStatusVariables")]
pub struct UpdateConnectorCurrentStatus {
    #[arguments(input: $input)]
    pub update_connector_current_status: Option<ManagedConnector>,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum ConnectorCurrentStatus {
    #[cynic(rename = "started")]
    Started,
    #[cynic(rename = "stopped")]
    Stopped,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum ConnectorRequestStatus {
    #[cynic(rename = "starting")]
    Starting,
    #[cynic(rename = "stopping")]
    Stopping,
}

#[derive(cynic::InputObject, Debug)]
pub struct CurrentConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub status: ConnectorCurrentStatus,
}
//endregion

pub async fn status(id: String, status: ConnectorStatus, api: &ApiOpenCTI) -> Option<ApiConnector> {
    use cynic::MutationBuilder;

    let update_status = match status {
        ConnectorStatus::Started => ConnectorCurrentStatus::Started,
        _ => ConnectorCurrentStatus::Stopped,
    };

    let vars = UpdateConnectorCurrentStatusVariables {
        input: CurrentConnectorStatusInput {
            id: &cynic::Id::new(id),
            status: update_status,
        },
    };
    let mutation = UpdateConnectorCurrentStatus::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            handle_graphql_response(
                response,
                "update_connector_current_status",
                "OpenCTI backend does not support XTM composer status updates. The connector will continue to run but status won't be updated in OpenCTI."
            ).and_then(|data| {
                extract_optional_field(
                    data.update_connector_current_status,
                    "update_connector_current_status",
                    "update_connector_current_status"
                ).map(|connector| connector.to_api_connector(&api.private_key))
            })
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to modify status");
            None
        }
    }
}
