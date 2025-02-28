use crate::api::engine::query_fetch;
use crate::config::settings::Settings;
use serde::Serialize;
use std::str::FromStr;

#[cynic::schema("opencti")]
mod schema {}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Connector {
    pub id: cynic::Id,
    pub name: String,
    #[cynic(rename = "manager_contract_image")]
    pub manager_contract_image: Option<String>,
    #[cynic(rename = "manager_current_status")]
    pub manager_current_status: Option<String>,
    #[cynic(rename = "manager_requested_status")]
    pub manager_requested_status: Option<String>,
    #[cynic(rename = "manager_contract_configuration")]
    pub manager_contract_configuration: Option<Vec<ConnectorContractConfiguration>>,
}

impl Connector {
    pub fn container_name(&self) -> String {
        self.name
            .clone()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .to_lowercase()
    }

    pub fn container_envs(&self) -> Vec<String> {
        self.manager_contract_configuration
            .clone()
            .unwrap()
            .into_iter()
            .map(|config| format!("{}={}", config.key, config.value))
            .collect::<Vec<String>>()
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
    pub connectors_for_manager: Option<Vec<Connector>>,
}

#[derive(cynic::QueryFragment, Debug, Clone, Serialize)]
pub struct ConnectorContractConfiguration {
    pub key: String,
    pub value: String,
}

pub async fn list(settings_data: &Settings) -> cynic::GraphQlResponse<GetConnectors> {
    use cynic::QueryBuilder;
    let manager_id = settings_data.manager.id.clone();
    let vars = GetConnectorsVariables {
        manager_id: (&manager_id).into(),
    };
    let query = GetConnectors::build(vars);
    query_fetch(settings_data, query).await
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
    pub update_connector_current_status: Option<Connector>,
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

pub async fn update_current_status(
    settings_data: &Settings,
    connector_id: String,
    status: ConnectorCurrentStatus,
) -> Option<Connector> {
    use cynic::MutationBuilder;
    let vars = UpdateConnectorCurrentStatusVariables {
        input: CurrentConnectorStatusInput {
            id: &cynic::Id::new(connector_id),
            status,
        },
    };
    let mutation = UpdateConnectorCurrentStatus::build(vars);
    let mutation_response = query_fetch(settings_data, mutation).await;
    mutation_response
        .data
        .unwrap()
        .update_connector_current_status
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
    pub update_connector_logs: Option<Connector>,
}

#[derive(cynic::InputObject, Debug)]
pub struct LogsConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub logs: Vec<&'a str>,
}

pub async fn update_connector_logs(
    settings_data: &Settings,
    connector_id: String,
    logs: Vec<String>,
) -> Option<Connector> {
    use cynic::MutationBuilder;
    let str_logs = logs.iter().map(|c| c.as_str()).collect();
    let vars = ReportConnectorLogsVariables {
        input: LogsConnectorStatusInput {
            id: &cynic::Id::new(connector_id),
            logs: str_logs,
        },
    };
    let mutation = ReportConnectorLogs::build(vars);
    let mutation_response = query_fetch(settings_data, mutation).await;
    mutation_response.data.unwrap().update_connector_logs
}
// endregion
