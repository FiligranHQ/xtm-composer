use cynic::GraphQlResponse;
use tracing::error;

/// Generic error handler for GraphQL responses
/// Returns the data if successful, None if there are errors or no data
pub fn handle_graphql_response<T>(
    response: GraphQlResponse<T>,
    operation_name: &str,
    unsupported_message: &str,
) -> Option<T> {
    // Check for GraphQL errors first
    let query_errors = response.errors.unwrap_or_default();
    if !query_errors.is_empty() {
        let errors: Vec<String> = query_errors.iter().map(|err| err.to_string()).collect();
        error!(
            error = errors.join(","),
            operation = operation_name,
            "GraphQL operation failed"
        );
        return None;
    }

    // Check if data is present
    match response.data {
        Some(data) => Some(data),
        None => {
            error!(operation = operation_name, "{}", unsupported_message);
            None
        }
    }
}

/// Helper to extract a nested optional field with error handling
pub fn extract_optional_field<T>(field: Option<T>, field_name: &str, operation_name: &str) -> Option<T> {
    match field {
        Some(value) => Some(value),
        None => {
            error!(
                operation = operation_name,
                field = field_name,
                "OpenCTI backend returned null for field"
            );
            None
        }
    }
}
