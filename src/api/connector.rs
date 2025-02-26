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

#[derive(cynic::QueryFragment, Debug)]
pub struct Connector {
    pub active: Option<bool>,
    #[cynic(rename = "connector_state")]
    pub connector_state: Option<String>,
    #[cynic(rename = "connector_type")]
    pub connector_type: Option<String>,
}

pub async fn list(settings_data: &Settings) -> cynic::GraphQlResponse<GetConnectors> {
    use cynic::QueryBuilder;
    let manager_id = settings_data.manager.id.clone();
    let vars = GetConnectorsVariables { manager_id: (&manager_id).into() };
    let query = GetConnectors::build(vars);
    query_fetch(settings_data, query).await
}