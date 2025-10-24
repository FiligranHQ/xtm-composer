use crate::api::opencti::ApiOpenCTI;
use crate::api::opencti::error_handler::{handle_graphql_response, extract_optional_field};
use tracing::{error, debug};

// region schema
use crate::api::opencti::opencti as schema;
use cynic;

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "Query")]
pub struct GetProxyConfiguration {
    #[cynic(rename = "connectorProxyConfiguration")]
    pub connector_proxy_configuration: Option<ProxyConfiguration>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ProxyConfiguration {
    #[cynic(rename = "http_proxy")]
    pub http_proxy: Option<ProxyUrlConfig>,
    #[cynic(rename = "https_proxy")]
    pub https_proxy: Option<HttpsProxyConfig>,
    #[cynic(rename = "no_proxy")]
    pub no_proxy: Vec<String>,
    #[cynic(rename = "exclusion_patterns")]
    pub exclusion_patterns: ExclusionPatterns,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ProxyUrlConfig {
    pub url: String,
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct HttpsProxyConfig {
    pub url: String,
    #[cynic(rename = "ca_certificates")]
    pub ca_certificates: Vec<String>,
    #[cynic(rename = "reject_unauthorized")]
    pub reject_unauthorized: bool,
    pub enabled: bool,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ExclusionPatterns {
    pub hostnames: Vec<String>,
    #[cynic(rename = "ip_ranges")]
    pub ip_ranges: Vec<String>,
    pub wildcards: Vec<String>,
}
// endregion

pub async fn get_proxy_configuration(api: &ApiOpenCTI) -> Option<ProxyConfiguration> {
    use cynic::QueryBuilder;

    debug!("Fetching proxy configuration from OpenCTI");
    
    let query = GetProxyConfiguration::build({});
    let response = api.query_fetch(query).await;
    
    match response {
        Ok(response) => {
            handle_graphql_response(
                response,
                "connector_proxy_configuration",
                "OpenCTI backend does not support proxy configuration. The composer will continue without proxy support."
            ).and_then(|data| {
                extract_optional_field(
                    data.connector_proxy_configuration,
                    "connector_proxy_configuration",
                    "connector_proxy_configuration"
                )
            })
        }
        Err(e) => {
            error!(error = e.to_string(), "Failed to fetch proxy configuration");
            None
        }
    }
}
