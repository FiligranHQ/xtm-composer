use cynic::Id;
use crate::api::engine::query_fetch;
use crate::config::settings::Settings;

#[cynic::schema("opencti")]
mod schema {}

#[derive(cynic::QueryVariables, Debug)]
pub struct GetConnectorsVariables<'a> {
    pub manager_id: &'a Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", variables = "GetConnectorsVariables")]
pub struct GetConnectors {
    #[arguments(managerId: $manager_id)]
    pub connectors_for_manager: Option<Vec<Connector>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct Connector {
    pub id: Id,
    pub name: String,
    #[cynic(rename = "manager_status")]
    pub manager_status: Option<String>,
    #[cynic(rename = "manager_contract_configuration")]
    pub manager_contract_configuration: Option<Vec<ConnectorContractConfiguration>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ConnectorContractConfiguration {
    pub key: String,
    pub value: String,
}

pub async fn list(settings_data: &Settings) -> cynic::GraphQlResponse<GetConnectors> {
    use cynic::QueryBuilder;
    let manager_id = settings_data.manager.id.clone();
    let vars = GetConnectorsVariables { manager_id: (&manager_id).into() };
    let query = GetConnectors::build(vars);
    query_fetch(settings_data, query).await
}