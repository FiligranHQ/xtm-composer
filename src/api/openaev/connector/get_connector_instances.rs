use crate::api::ApiConnector;
use crate::api::openaev::api_handler::handle_api_response;
use crate::api::openaev::connector::ConnectorInstances;

pub async fn get_connector_instances(api: &crate::api::openaev::ApiOpenAEV) -> Option<Vec<ApiConnector>> {
    let settings = crate::settings();
    let get_connectors = api.get(&format!("/xtm-composer/{}/connector-instances", settings.manager.id))
        .send()
        .await;

    handle_api_response::<Vec<ConnectorInstances>>(get_connectors, "fetch connector instances")
        .await.map(|connectors| {
        connectors
            .into_iter()
            .map(|connector| connector.to_api_connector(&api.private_key))
            .collect()
    })
}