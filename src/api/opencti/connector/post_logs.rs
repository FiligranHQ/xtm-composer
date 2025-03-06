use tracing::error;
use crate::api::ApiConnector;
use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::connector::ManagedConnector;

use cynic;
use crate::api::opencti::opencti as schema;

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
    let query_data = mutation_response.data.unwrap();
    let query_errors = mutation_response.errors.unwrap_or_default();
    if !query_errors.is_empty() {
        let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
        error!(error = errors.join(","), "Fail to patch logs");
        None
    } else {
        let connector = query_data.update_connector_logs.unwrap();
        Some(connector.to_api_connector())
    }
}