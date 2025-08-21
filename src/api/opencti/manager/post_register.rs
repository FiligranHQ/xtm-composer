use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::manager::ConnectorManager;
use crate::api::opencti::error_handler::{handle_graphql_response, extract_optional_field};
use crate::api::opencti::opencti as schema;
use cynic;
use tracing::{error, info};
use rsa::{RsaPublicKey, RsaPrivateKey, pkcs1::DecodeRsaPrivateKey, pkcs1::EncodeRsaPublicKey};

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
    pub public_key: &'a str,
}
// endregion

pub async fn register(api: &ApiOpenCTI) {
    use cynic::MutationBuilder;

    let settings = crate::settings();
    let priv_key = RsaPrivateKey::from_pkcs1_pem(&settings.manager.credentials_key).unwrap();
    let pub_key = RsaPublicKey::from(&priv_key);
    let public_key = RsaPublicKey::to_pkcs1_pem(&pub_key, Default::default()).unwrap();

    let vars = RegisterConnectorsManageVariables {
        input: RegisterConnectorsManagerInput {
            id: &cynic::Id::new(&settings.manager.id),
            name: &settings.manager.name,
            public_key: &public_key,
        },
    };
    let mutation = RegisterConnectorsManager::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            if let Some(data) = handle_graphql_response(
                response,
                "register_connectors_manager",
                "OpenCTI backend does not support XTM composer manager registration. The composer will continue to run but won't be registered in OpenCTI."
            ) {
                if let Some(manager) = extract_optional_field(
                    data.register_connectors_manager,
                    "register_connectors_manager",
                    "register_connectors_manager"
                ) {
                    info!(manager_id = manager.id.into_inner(), "Manager registered");
                }
            }
        }
        Err(e) => {
            error!(error = e.to_string(), "Error registering connector manager");
        }
    }
}
