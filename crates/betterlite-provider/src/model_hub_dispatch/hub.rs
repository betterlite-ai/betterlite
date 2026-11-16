use super::HubDispatchError;
use crate::operation::OperationError;
use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::huggingface_chat::error::HuggingfaceError;
use betterlite_openai::huggingface_chat::mapping::{HubGet, HubResponse};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Json;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use std::time::Duration;
use url::Url;
pub const HUB_URL: &str = "https://huggingface.co";
const EXPAND: (&str, &str) = ("expand", "inferenceProviderMapping");
const PIPELINE_TAG: &str = "pipeline_tag";
#[async_trait]
pub trait HubGetAsync: Send + Sync {
    async fn get(
        &self,
        url: &str,
        headers: &Headers,
        query: &[(&str, &str)],
    ) -> Result<HubResponse, HubDispatchError>;
}
pub struct PooledHub<'client> {
    pub client: &'client ProxiedClient,
    pub deadline: Duration,
}
#[async_trait]
impl HubGetAsync for PooledHub<'_> {
    async fn get(
        &self,
        url: &str,
        headers: &Headers,
        query: &[(&str, &str)],
    ) -> Result<HubResponse, HubDispatchError> {
        let response = self
            .client
            .send_buffered(OutboundRequest {
                method: Method::GET,
                url: Url::parse(url).map_err(|source| HubDispatchError::Url {
                    url: url.to_owned(),
                    source,
                })?,
                query: query
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
                headers: header_map(headers)
                    .map_err(|source| HubDispatchError::Headers { source })?,
                body: RequestBody::Empty,
                deadline: self.deadline,
            })
            .await
            .map_err(OperationError::from)?;
        Ok(HubResponse {
            status_code: response.status,
            headers: headers_from(response.headers),
            body: response.body,
        })
    }
}
fn headers_from(headers: http::HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let text = value.to_str().ok()?;
            Some((name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}
fn mapping_headers(token: Option<&SecretString>) -> Headers {
    let mut headers = Headers::from([(String::from("Accept"), String::from("application/json"))]);
    let supplied = token.filter(|token| !token.expose_secret().is_empty());
    if let Some(token) = supplied {
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", token.expose_secret()),
        );
    }
    headers
}
pub async fn fetch_mapping(
    hub: &dyn HubGetAsync,
    model_id: &str,
    token: Option<&SecretString>,
) -> Result<HubResponse, HubDispatchError> {
    let url = format!("{HUB_URL}/api/models/{model_id}");
    hub.get(&url, &mapping_headers(token), &[EXPAND]).await
}
pub async fn fetch_pipeline_tag(
    hub: &dyn HubGetAsync,
    model: &str,
) -> Result<HubResponse, HubDispatchError> {
    let url = format!("{HUB_URL}/api/models/{model}");
    hub.get(&url, &Headers::new(), &[]).await
}
pub struct PrefetchedHub(pub HubResponse);
impl HubGet for PrefetchedHub {
    fn get(
        &self,
        _url: &str,
        _headers: &Headers,
        _query: &[(&str, &str)],
    ) -> Result<HubResponse, betterlite_http::proxy::transport_error::TransportError> {
        Ok(self.0.clone())
    }
}
pub struct PrefetchedTag(pub HubResponse);
impl PipelineTagSource for PrefetchedTag {
    fn tag(
        &self,
        _model: &str,
        _task_type: Option<&str>,
    ) -> Result<Option<String>, HuggingfaceError> {
        let value: Json =
            serde_json::from_slice(&self.0.body).map_err(|_error| HuggingfaceError {
                status_code: 500,
                message: String::from("the Hugging Face model info is not JSON"),
                headers: Headers::new(),
            })?;
        Ok(value
            .as_object()
            .and_then(|object| object.get(PIPELINE_TAG))
            .and_then(Json::as_str)
            .map(str::to_owned))
    }
}
