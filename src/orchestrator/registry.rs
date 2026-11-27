//! Registry lookups: the digest a moving tag (`:rolling`, `:latest`) points to
//! now, read with a `HEAD` on its manifest (Docker Registry HTTP API v2), so a
//! workload can be redeployed when a new image is pushed under its tag (#237).
use crate::orchestrator::image::Image;
use base64::Engine;
use base64::engine::general_purpose;
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use std::time::Duration;

/// Manifest media types accepted, index first: the digest of a multi-platform
/// image is the digest of its index, the one engines report as repo digest.
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
application/vnd.docker.distribution.manifest.list.v2+json, \
application/vnd.oci.image.manifest.v1+json, \
application/vnd.docker.distribution.manifest.v2+json";

const DIGEST_HEADER: &str = "docker-content-digest";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);
const DOCKER_HUB: &str = "registry-1.docker.io";

/// Tags that are republished in place: a workload on one of them must follow
/// the image the tag points to. Image tag suffixes (`-fips`, `-ubi9`) are ignored.
const MOVING_TAGS: [&str; 2] = ["latest", "rolling"];
const TAG_SUFFIXES: [&str; 2] = ["-fips", "-ubi9"];

/// True when the image is on a moving tag: `:latest`, `:rolling` or no tag.
/// A pinned version (`6.9.0`) or a digest (`@sha256:...`) never moves.
pub fn is_moving_tag(image: &str) -> bool {
    if image.contains('@') {
        return false;
    }
    let name_start = image.rfind('/').map_or(0, |index| index + 1);
    match image[name_start..].rfind(':') {
        None => true,
        Some(index) => {
            let tag = &image[name_start + index + 1..];
            let tag = TAG_SUFFIXES
                .iter()
                .find_map(|suffix| tag.strip_suffix(suffix))
                .unwrap_or(tag);
            MOVING_TAGS.contains(&tag)
        }
    }
}

/// An image reference split into the registry host, the repository and the tag.
#[derive(Debug, PartialEq)]
pub struct Reference {
    pub host: String,
    pub repository: String,
    pub tag: String,
}

/// Parse `[host[:port]/]repository[:tag]`. None for a reference by digest.
pub fn parse_reference(image: &str) -> Option<Reference> {
    if image.contains('@') {
        return None;
    }
    let name_start = image.rfind('/').map_or(0, |index| index + 1);
    let (name, tag) = match image[name_start..].rfind(':') {
        Some(index) => (
            &image[..name_start + index],
            image[name_start + index + 1..].to_string(),
        ),
        None => (image, "latest".to_string()),
    };
    let (host, repository) = match name.split_once('/') {
        Some((first, rest))
            if first.contains('.') || first.contains(':') || first == "localhost" =>
        {
            (first.to_string(), rest.to_string())
        }
        _ => (DOCKER_HUB.to_string(), name.to_string()),
    };
    let host = match host.as_str() {
        "docker.io" | "index.docker.io" => DOCKER_HUB.to_string(),
        _ => host,
    };
    // Official Docker Hub images live under library/
    let repository = if host == DOCKER_HUB && !repository.contains('/') {
        format!("library/{repository}")
    } else {
        repository
    };
    Some(Reference {
        host,
        repository,
        tag,
    })
}

/// Parameters of a `Bearer realm="...",service="...",scope="..."` challenge.
fn challenge_parameters(challenge: &str) -> Vec<(String, String)> {
    let mut parameters = Vec::new();
    let mut rest = challenge.trim();
    while !rest.is_empty() {
        let Some((key, value_start)) = rest.split_once('=') else {
            break;
        };
        let key = key
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let (value, remaining) = match value_start.strip_prefix('"') {
            Some(quoted) => match quoted.split_once('"') {
                Some((value, remaining)) => (value.to_string(), remaining),
                None => (quoted.to_string(), ""),
            },
            None => match value_start.split_once(',') {
                Some((value, remaining)) => (value.trim().to_string(), remaining),
                None => (value_start.trim().to_string(), ""),
            },
        };
        parameters.push((key, value));
        rest = remaining.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    }
    parameters
}

fn basic_authorization(credentials: &(String, String)) -> String {
    let (username, password) = credentials;
    format!(
        "Basic {}",
        general_purpose::STANDARD.encode(format!("{username}:{password}"))
    )
}

/// The Authorization header answering a registry challenge: the configured
/// credentials for a Basic challenge, a token from the realm for a Bearer one
/// (anonymous when no credentials are configured, as for public images).
async fn authorize(
    client: &Client,
    challenge: &str,
    credentials: Option<&(String, String)>,
) -> Result<String, String> {
    let (scheme, parameters) = challenge.trim().split_once(' ').unwrap_or((challenge, ""));
    if scheme.eq_ignore_ascii_case("basic") {
        return credentials
            .map(basic_authorization)
            .ok_or_else(|| "the registry requires credentials".to_string());
    }
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(format!(
            "unsupported registry authentication \"{challenge}\""
        ));
    }
    let parameters = challenge_parameters(parameters);
    let realm = parameters
        .iter()
        .find(|(key, _)| key == "realm")
        .map(|(_, value)| value.clone())
        .ok_or_else(|| "the registry challenge has no realm".to_string())?;
    let query: Vec<(String, String)> = parameters
        .into_iter()
        .filter(|(key, _)| key == "service" || key == "scope")
        .collect();
    let mut request = client.get(&realm).query(&query);
    if let Some(credentials) = credentials {
        request = request.header(AUTHORIZATION, basic_authorization(credentials));
    }
    let response = request.send().await.map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "the registry token service answered {}",
            response.status()
        ));
    }
    let body: serde_json::Value = response.json().await.map_err(|err| err.to_string())?;
    body.get("token")
        .or_else(|| body.get("access_token"))
        .and_then(|token| token.as_str())
        .map(|token| format!("Bearer {token}"))
        .ok_or_else(|| "the registry token service returned no token".to_string())
}

async fn head_manifest(
    client: &Client,
    url: &str,
    authorization: Option<&str>,
) -> Result<Response, String> {
    let mut request = client.head(url).header(ACCEPT, MANIFEST_ACCEPT);
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    request.send().await.map_err(|err| err.to_string())
}

/// Digest the registry at `base_url` serves for the reference, as returned in
/// the `Docker-Content-Digest` header of a `HEAD` on the manifest (no pull, and
/// not counted by Docker Hub as a pull).
pub(crate) async fn manifest_digest(
    client: &Client,
    base_url: &str,
    reference: &Reference,
    credentials: Option<&(String, String)>,
) -> Result<String, String> {
    let url = format!(
        "{base_url}/v2/{}/manifests/{}",
        reference.repository, reference.tag
    );
    let mut response = head_manifest(client, &url, None).await?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get(WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let authorization = authorize(client, &challenge, credentials).await?;
        response = head_manifest(client, &url, Some(&authorization)).await?;
    }
    if !response.status().is_success() {
        return Err(format!(
            "the registry answered {} for {}",
            response.status(),
            url
        ));
    }
    response
        .headers()
        .get(DIGEST_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| "the registry returned no manifest digest".to_string())
}

/// Digest the workload image currently points to in its registry, resolved
/// with the platform registry and image tag suffix like a pull. None for an
/// image that does not move (pinned version or digest): it is never looked up.
pub async fn resolve_digest(resolver: &Image, image: &str) -> Result<Option<String>, String> {
    if !is_moving_tag(image) {
        return Ok(None);
    }
    let Some(reference) = parse_reference(&resolver.build_name(image.to_string())) else {
        return Ok(None);
    };
    let client = Client::builder()
        .timeout(LOOKUP_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())?;
    let base_url = format!("https://{}", reference.host);
    manifest_digest(
        &client,
        &base_url,
        &reference,
        resolver.basic_credentials().as_ref(),
    )
    .await
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    const DIGEST: &str = "sha256:5d2f4b0a8a5e4c3d2b1a09f8e7d6c5b4a3928170f6e5d4c3b2a1908f7e6d5c4b";

    #[test]
    fn moving_tags_are_latest_rolling_or_no_tag() {
        assert!(is_moving_tag("openaev/collector-mitre-attack:rolling"));
        assert!(is_moving_tag("opencti/connector-mitre:latest"));
        assert!(is_moving_tag("opencti/connector-mitre"));
        assert!(is_moving_tag("opencti/connector-mitre:rolling-fips"));
        assert!(is_moving_tag("localhost:5000/connector-mitre"));
        assert!(!is_moving_tag("opencti/connector-mitre:6.9.0"));
        assert!(!is_moving_tag("opencti/connector-mitre:6.9.0-fips"));
        assert!(!is_moving_tag(&format!(
            "opencti/connector-mitre:rolling@{DIGEST}"
        )));
        assert!(!is_moving_tag("localhost:5000/connector-mitre:6.9.0"));
    }

    fn reference(host: &str, repository: &str, tag: &str) -> Option<Reference> {
        Some(Reference {
            host: host.to_string(),
            repository: repository.to_string(),
            tag: tag.to_string(),
        })
    }

    #[test]
    fn references_are_split_into_host_repository_and_tag() {
        assert_eq!(
            parse_reference("openaev/collector-mitre-attack:rolling"),
            reference(DOCKER_HUB, "openaev/collector-mitre-attack", "rolling")
        );
        assert_eq!(
            parse_reference("redis"),
            reference(DOCKER_HUB, "library/redis", "latest")
        );
        assert_eq!(
            parse_reference("docker.io/opencti/connector-mitre:latest"),
            reference(DOCKER_HUB, "opencti/connector-mitre", "latest")
        );
        assert_eq!(
            parse_reference("ghcr.io/acme/test:latest"),
            reference("ghcr.io", "acme/test", "latest")
        );
        assert_eq!(
            parse_reference("registry.example.com:5000/ghcr.io/acme/test"),
            reference("registry.example.com:5000", "ghcr.io/acme/test", "latest")
        );
        assert_eq!(parse_reference(&format!("acme/test@{DIGEST}")), None);
    }

    #[test]
    fn bearer_challenge_parameters_are_read_with_quoted_commas() {
        assert_eq!(
            challenge_parameters(
                r#"realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:acme/test:pull,push""#
            ),
            vec![
                (
                    "realm".to_string(),
                    "https://auth.docker.io/token".to_string()
                ),
                ("service".to_string(), "registry.docker.io".to_string()),
                (
                    "scope".to_string(),
                    "repository:acme/test:pull,push".to_string()
                ),
            ]
        );
    }

    /// Fake registry answering one response per connection, in order, and
    /// recording the head of each request it received.
    async fn serve(listener: TcpListener, responses: Vec<String>) -> Arc<Mutex<Vec<String>>> {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        tokio::spawn(async move {
            for response in responses {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut raw = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    raw.push_str(&line);
                }
                captured.lock().unwrap().push(raw);
                reader
                    .into_inner()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        requests
    }

    fn response(status: &str, headers: &[(&str, String)], body: &str) -> String {
        let headers: String = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn acme_test() -> Reference {
        parse_reference("ghcr.io/acme/test:rolling").unwrap()
    }

    #[tokio::test]
    async fn the_digest_is_read_after_a_bearer_token_exchange() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let challenge = format!(
            r#"Bearer realm="{base_url}/token",service="registry.test",scope="repository:acme/test:pull""#
        );
        let requests = serve(
            listener,
            vec![
                response("401 Unauthorized", &[("WWW-Authenticate", challenge)], ""),
                response(
                    "200 OK",
                    &[("Content-Type", "application/json".to_string())],
                    r#"{"token":"t0k3n"}"#,
                ),
                response(
                    "200 OK",
                    &[("Docker-Content-Digest", DIGEST.to_string())],
                    "",
                ),
            ],
        )
        .await;
        let credentials = ("user".to_string(), "secret".to_string());

        let digest = manifest_digest(&Client::new(), &base_url, &acme_test(), Some(&credentials))
            .await
            .unwrap();

        assert_eq!(digest, DIGEST);
        let requests = requests.lock().unwrap().clone();
        assert!(
            requests[0].starts_with("HEAD /v2/acme/test/manifests/rolling "),
            "{}",
            requests[0]
        );
        let token_request = requests[1].to_ascii_lowercase();
        assert!(
            token_request.starts_with(
                "get /token?service=registry.test&scope=repository%3aacme%2ftest%3apull "
            ),
            "{token_request}"
        );
        assert!(
            token_request.contains("authorization: basic dxnlcjpzzwnyzxq="),
            "{token_request}"
        );
        assert!(
            requests[2]
                .to_ascii_lowercase()
                .contains("authorization: bearer t0k3n"),
            "{}",
            requests[2]
        );
        assert!(
            requests[2]
                .to_ascii_lowercase()
                .contains("application/vnd.oci.image.index.v1+json")
        );
    }

    #[tokio::test]
    async fn a_missing_tag_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        serve(listener, vec![response("404 Not Found", &[], "")]).await;

        let error = manifest_digest(&Client::new(), &base_url, &acme_test(), None)
            .await
            .unwrap_err();

        assert!(error.contains("404"), "{error}");
    }

    #[tokio::test]
    async fn a_pinned_image_is_never_looked_up() {
        let resolver = Image::new(None, None);
        // No registry answers: a lookup would fail
        assert_eq!(
            resolve_digest(&resolver, "opencti/connector-mitre:6.9.0").await,
            Ok(None)
        );
        assert_eq!(
            resolve_digest(&resolver, &format!("opencti/connector-mitre@{DIGEST}")).await,
            Ok(None)
        );
    }
}
