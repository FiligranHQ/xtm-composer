use crate::api::{ApiConnector, ConnectorStatus};
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;

use cynic;
use crate::api::opencti::opencti as schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateConnectorCurrentStatusVariables<'a> {
    pub input: CurrentConnectorStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    variables = "UpdateConnectorCurrentStatusVariables"
)]
pub struct UpdateConnectorCurrentStatus {
    #[arguments(input: $input)]
    pub update_connector_current_status: Option<ManagedConnector>,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum ConnectorCurrentStatus {
    #[cynic(rename = "created")]
    Created,
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

pub async fn patch_status(
    connector_id: String,
    status: ConnectorStatus,
    api: &ApiOpenCTI,
) -> Option<ApiConnector> {
    use cynic::MutationBuilder;

    let update_status = match status {
        ConnectorStatus::Started => ConnectorCurrentStatus::Started,
        _ => ConnectorCurrentStatus::Stopped,
    };

    let vars = UpdateConnectorCurrentStatusVariables {
        input: CurrentConnectorStatusInput {
            id: &cynic::Id::new(connector_id),
            status: update_status,
        },
    };
    let mutation = UpdateConnectorCurrentStatus::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    let connector = mutation_response
        .data
        .unwrap()
        .update_connector_current_status
        .unwrap();
    Some(connector.to_api_connector())
}