use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::manager::ConnectorManager;
use crate::api::opencti::opencti as schema;
use cynic;
use tracing::{error, info};

// region schema
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

#[derive(cynic::InputObject, Debug)]
pub struct RegisterConnectorsManagerInput<'a> {
    pub id: &'a cynic::Id,
    pub name: &'a str,
}
// endregion

pub async fn register(api: &ApiOpenCTI) {
    use cynic::MutationBuilder;

    let settings = crate::settings();
    let vars = RegisterConnectorsManageVariables {
        input: RegisterConnectorsManagerInput {
            id: &cynic::Id::new(&settings.manager.id),
            name: &settings.manager.name,
        },
    };
    let mutation = RegisterConnectorsManager::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            let query_errors = response.errors.unwrap_or_default();
            if !query_errors.is_empty() {
                let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
                error!(
                    error = errors.join(","),
                    "Error registering connector manager"
                );
            } else {
                let data = response.data.unwrap().register_connectors_manager.unwrap();
                info!(manager_id = data.id.into_inner(), "Manager registered");
            }
        }
        Err(e) => {
            error!(error = e.to_string(), "Error registering connector manager");
        }
    }
}
