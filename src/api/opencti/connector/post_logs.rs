use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::error_handler::handle_graphql_response;
use tracing::error;

// region schema
use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryVariables, Debug)]
pub struct ReportConnectorLogsVariables<'a> {
    pub input: LogsConnectorStatusInput<'a>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "ReportConnectorLogsVariables")]
pub struct ReportConnectorLogs {
    #[arguments(input: $input)]
    pub update_connector_logs: cynic::Id,
}

#[derive(cynic::InputObject, Debug)]
pub struct LogsConnectorStatusInput<'a> {
    pub id: &'a cynic::Id,
    pub logs: Vec<&'a str>,
}
// endregion

pub async fn logs(id: String, logs: Vec<String>, api: &ApiOpenCTI) -> Option<cynic::Id> {
    use cynic::MutationBuilder;
    let str_logs = logs.iter().map(|c| c.as_str()).collect();
    let vars = ReportConnectorLogsVariables {
        input: LogsConnectorStatusInput {
            id: &cynic::Id::new(id),
            logs: str_logs,
        },
    };
    let mutation = ReportConnectorLogs::build(vars);
    let mutation_response = api.query_fetch(mutation).await;
    match mutation_response {
        Ok(response) => {
            handle_graphql_response(
                response,
                "update_connector_logs",
                "OpenCTI backend does not support XTM composer log updates. The connector will continue to run but logs won't be sent to OpenCTI."
            ).map(|data| data.update_connector_logs)
        }
        Err(e) => {
            error!(error = e.to_string(), "Fail to push logs");
            None
        }
    }
}
