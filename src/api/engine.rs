use cynic::Operation;
use serde::de::DeserializeOwned;
use serde::Serialize;
use crate::config::settings::Settings;

const BEARER: &str = "Bearer";
const AUTHORIZATION_HEADER: &str = "Authorization";

pub async fn query_fetch<R, V>(settings_data: &Settings, query: Operation<R, V>) -> cynic::GraphQlResponse<R> 
    where V: Serialize, R: DeserializeOwned + 'static {
    use cynic::http::ReqwestExt;
    let bearer = format!("{} {}", BEARER, settings_data.opencti.token);
    reqwest::Client::builder().build().unwrap().post(&settings_data.opencti.url)
        .header(AUTHORIZATION_HEADER, bearer.as_str())
        .run_graphql(query).await.unwrap()
}
