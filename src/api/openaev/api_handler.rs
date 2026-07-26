use serde::de::DeserializeOwned;
use tracing::error;

pub async fn handle_api_response<T>(
    response: Result<reqwest::Response, reqwest::Error>,
    operation_name: &str,
) -> Option<T>
where
    T: DeserializeOwned,
{
    match response {
        Ok(resp) if resp.status().is_success() => match resp.json::<T>().await {
            Ok(data) => Some(data),
            Err(err) => {
                error!(
                    "Failed to parse JSON for {}: {}",
                    operation_name,
                    err.to_string()
                );
                None
            }
        },
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

/// Handles API responses for acknowledgment-only endpoints whose success
/// body is empty or non-JSON (e.g. void endpoints, plain-text acks).
/// Only the HTTP status is checked; the body is drained and discarded.
pub async fn handle_api_status_response(
    response: Result<reqwest::Response, reqwest::Error>,
    operation_name: &str,
) -> Option<()> {
    match response {
        Ok(resp) if resp.status().is_success() => {
            // Drain the body so the underlying connection
            // can be returned to the pool and reused.
            let _ = resp.bytes().await;
            Some(())
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
        Ok(resp) if resp.status().is_success() => resp.text().await.ok().or_else(|| {
            error!("Failed to read response body for {}", operation_name);
            None
        }),
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

#[cfg(test)]
mod tests {
    use super::handle_api_status_response;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Use no_proxy() to avoid picking up HTTP_PROXY env vars mutated by parallel tests.
    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn spawn_server(status_line: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        format!("http://127.0.0.1:{}/", addr.port())
    }

    /// Regression test: the logs endpoint may return an empty body on success.
    /// handle_api_status_response must succeed on any 2xx regardless of body content.
    #[tokio::test]
    async fn handle_api_status_response_accepts_empty_success_body() {
        let url = spawn_server("HTTP/1.1 200 OK", "").await;
        let response = no_proxy_client().post(url).send().await;
        let result = handle_api_status_response(response, "push logs for connector instance").await;
        assert!(
            result.is_some(),
            "Expected Some(()) for a 200 response with an empty body"
        );
    }

    /// Regression test: a non-JSON (plain text) success body must not be
    /// treated as an error either.
    #[tokio::test]
    async fn handle_api_status_response_accepts_non_json_success_body() {
        let url = spawn_server("HTTP/1.1 200 OK", "ack").await;
        let response = no_proxy_client().post(url).send().await;
        let result = handle_api_status_response(response, "push logs for connector instance").await;
        assert!(
            result.is_some(),
            "Expected Some(()) for a 200 response with a non-JSON body"
        );
    }

    #[tokio::test]
    async fn handle_api_status_response_returns_none_on_error_status() {
        let url = spawn_server("HTTP/1.1 500 Internal Server Error", "").await;
        let response = no_proxy_client().post(url).send().await;
        let result = handle_api_status_response(response, "push logs for connector instance").await;

        assert!(result.is_none(), "Expected None for a 500 response");
    }
}
