use serde::de::DeserializeOwned;
use tracing::error;

pub async fn handle_api_response<T>(
    response: Result<reqwest::Response, reqwest::Error>,
    operation_name: &str,
) -> Option<T>
where
    T: DeserializeOwned
{
    match response {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<T>().await {
                Ok(data) => Some(data),
                Err(err) => {
                    error!("Failed to parse JSON for {}: {}", operation_name, err.to_string());
                    None
                }
            }
        }
        Ok(resp) => {
            error!(
                status = resp.status().as_u16(),
                "Failed to {}: non-success status code", operation_name
            );
            None
        }
        Err(err) => {
            error!(
                error = err.to_string(),
                "Failed to {}, check your configuration", operation_name
            );
            None
        }
    }
}

pub async fn handle_api_text_response(
    response: Result<reqwest::Response, reqwest::Error>,
    operation_name: &str,
) -> Option<String> {
    match response {
        Ok(resp) if resp.status().is_success() => {
            resp.text().await.ok().or_else(|| {
                error!("Failed to read response body for {}", operation_name);
                None
            })
        }
        Ok(resp) => {
            error!(
                status = resp.status().as_u16(),
                "Failed to {}: non-success status code", operation_name
            );
            None
        }
        Err(err) => {
            error!(
                error = err.to_string(),
                "Failed to {}, check your configuration", operation_name
            );
            None
        }
    }
}
