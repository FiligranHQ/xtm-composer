use crate::config::settings::Settings;
use cynic::Operation;
use serde::de::DeserializeOwned;
use serde::Serialize;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

pub async fn query_fetch<R, V>(
    settings: &Settings,
    query: Operation<R, V>,
) -> cynic::GraphQlResponse<R>
where
    V: Serialize,
    R: DeserializeOwned + 'static,
{
    use cynic::http::ReqwestExt;
    let bearer = format!("{} {}", BEARER, settings.opencti.token);
    let api_uri = format!("{}/graphql", &settings.opencti.url);
    reqwest::Client::builder()
        .build()
        .unwrap()
        .post(api_uri)
        .header(AUTHORIZATION_HEADER, bearer.as_str())
        .run_graphql(query)
        .await
        .unwrap()
}
