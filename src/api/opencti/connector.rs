use crate::api::opencti::opencti::ApiOpenCTI;
use crate::api::{ApiConnector, ApiContractConfig};
use crate::config::settings::Settings;
use serde::Serialize;
use std::str::FromStr;

#[cynic::schema("opencti")]
mod schema {}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ManagedConnector {
    pub id: cynic::Id,
    pub name: String,
    #[cynic(rename = "manager_contract_hash")]
    pub manager_contract_hash: Option<String>,
    #[cynic(rename = "manager_contract_image")]
    pub manager_contract_image: Option<String>,
    #[cynic(rename = "manager_current_status")]
    pub manager_current_status: Option<String>,
    #[cynic(rename = "manager_requested_status")]
    pub manager_requested_status: Option<String>,
    #[cynic(rename = "manager_contract_configuration")]
    pub manager_contract_configuration: Option<Vec<ConnectorContractConfiguration>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvVariable {
    pub key: String,
    pub value: String,
}

impl ManagedConnector {
    pub fn to_api_connector(&self) -> ApiConnector {
        let contract_configuration = self
            .manager_contract_configuration
            .clone()
            .unwrap()
            .into_iter()
            .map(|c| ApiContractConfig {
                key: c.key,
                value: c.value,
            })
            .collect();
        ApiConnector {
            id: self.id.clone().into_inner(),
            name: self.name.clone(),
            image: self.manager_contract_image.clone().unwrap(),
            contract_hash: self.manager_contract_hash.clone().unwrap(),
            current_status: self.manager_current_status.clone(),
            requested_status: self.manager_requested_status.clone().unwrap(),
            contract_configuration,
        }
    }
}

// region listing
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

#[derive(cynic::QueryFragment, Debug, Clone, Serialize)]
pub struct ConnectorContractConfiguration {
    pub key: String,
    pub value: String,
}

pub async fn list(settings: &Settings, api: &ApiOpenCTI) -> Option<Vec<ApiConnector>> {
    use cynic::QueryBuilder;
    let manager_id = settings.manager.id.clone();
    let vars = GetConnectorsVariables {
        manager_id: (&manager_id).into(),
    };
    let query = GetConnectors::build(vars);
    let get_connectors = api.query_fetch(query).await;
    let connectors = get_connectors
        .data
        .unwrap()
        .connectors_for_manager
        .unwrap()
        .into_iter()
        .map(|managed_connector| managed_connector.to_api_connector())
        .collect();
    Some(connectors)
}
// endregion

// region report status
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

impl FromStr for ConnectorCurrentStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<ConnectorCurrentStatus, Self::Err> {
        match input {
            "created" => Ok(ConnectorCurrentStatus::Created),
            "exited" => Ok(ConnectorCurrentStatus::Stopped),
            "started" => Ok(ConnectorCurrentStatus::Started),
            "healthy" => Ok(ConnectorCurrentStatus::Started),
            "running" => Ok(ConnectorCurrentStatus::Started),
            _ => Ok(ConnectorCurrentStatus::Created),
        }
    }
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq)]
pub enum ConnectorRequestStatus {
    #[cynic(rename = "starting")]
    Starting,
    #[cynic(rename = "stopping")]
    Stopping,
}

impl FromStr for ConnectorRequestStatus {
    type Err = ();
    fn from_str(input: &str) -> Result<ConnectorRequestStatus, Self::Err> {
        match input {
            "starting" => Ok(ConnectorRequestStatus::Starting),
            "stopping" => Ok(ConnectorRequestStatus::Stopping),
            _ => Ok(ConnectorRequestStatus::Stopping),
        }
    }
}

#[derive(cynic::InputObject, Debug)]
pub struct CurrentConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub status: ConnectorCurrentStatus,
}

pub async fn patch_status(
    connector_id: String,
    status: ConnectorCurrentStatus,
    api: &ApiOpenCTI,
) -> Option<ApiConnector> {
    use cynic::MutationBuilder;
    let vars = UpdateConnectorCurrentStatusVariables {
        input: CurrentConnectorStatusInput {
            id: &cynic::Id::new(connector_id),
            status,
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
// endregion

// region report logs
#[derive(cynic::QueryVariables, Debug)]
pub struct ReportConnectorLogsVariables<'a> {
    pub input: LogsConnectorStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "ReportConnectorLogsVariables")]
pub struct ReportConnectorLogs {
    #[arguments(input: $input)]
    pub update_connector_logs: Option<ManagedConnector>,
}

#[derive(cynic::InputObject, Debug)]
pub struct LogsConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub logs: Vec<&'a str>,
}

pub async fn patch_logs(
    connector_id: String,
    logs: Vec<String>,
    api: &ApiOpenCTI,
) -> Option<ApiConnector> {
    use cynic::MutationBuilder;
    let str_logs = logs.iter().map(|c| c.as_str()).collect();
    let vars = ReportConnectorLogsVariables {
        input: LogsConnectorStatusInput {
            id: &cynic::Id::new(connector_id),
            logs: str_logs,
        },
    };
    let mutation = ReportConnectorLogs::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    let connector = mutation_response
        .data
        .unwrap()
        .update_connector_logs
        .unwrap();
    Some(connector.to_api_connector())
}
// endregion
