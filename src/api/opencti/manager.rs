use crate::api::opencti::opencti::ApiOpenCTI;
use crate::config::settings::Settings;
use std::fs;

#[cynic::schema("opencti")]
mod schema {}

#[derive(cynic::QueryVariables, Debug)]
pub struct RegisterConnectorsManageVariables<'a> {
    pub input: RegisterConnectorsManagerInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    variables = "RegisterConnectorsManageVariables"
)]
pub struct RegisterConnectorsManager {
    #[arguments(input: $input)]
    pub register_connectors_manager: Option<ConnectorManager>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ConnectorManager {
    pub id: cynic::Id,
}

#[derive(cynic::InputObject, Debug)]
pub struct RegisterConnectorsManagerInput<'a> {
    pub id: &'a cynic::Id,
    pub name: &'a str,
    pub contracts: Vec<&'a str>,
}

pub async fn register_manager(settings: &Settings, api: &ApiOpenCTI) -> Option<String> {
    use cynic::MutationBuilder;
    let directory = fs::read_dir("./contracts/opencti").unwrap();
    let contracts: Vec<String> = directory
        .map(|file| fs::read_to_string(file.unwrap().path()).unwrap())
        .collect();
    let contracts = contracts.iter().map(|content| content.as_str()).collect();
    let vars = RegisterConnectorsManageVariables {
        input: RegisterConnectorsManagerInput {
            id: &cynic::Id::new(&settings.manager.id),
            name: &settings.manager.name,
            contracts,
        },
    };
    let mutation = RegisterConnectorsManager::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    let response = mutation_response.data.unwrap().register_connectors_manager;
    match response {
        Some(_) => {
            Some(response.unwrap().id.into_inner())
        }
        None => {
            panic!("{:?}", mutation_response.errors.unwrap());
        }
    }
}
