use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::huggingface_chat::error::HuggingfaceError;
use betterlite_openai::huggingface_chat::mapping::{HubGet, HubResponse};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Json;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use super::transport::WireRequest;
use super::{DispatchDeps, HubDispatchError};
const HUB_URL: &str = "https://huggingface.co";
pub const EXPAND: (&str, &str) = ("expand", "inferenceProviderMapping");
const PIPELINE_TAG: &str = "pipeline_tag";
#[must_use]
pub fn headers_from(headers: &http::HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let text = value.to_str().ok()?;
            Some((name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}
#[must_use]
pub fn hub_response(response: UpstreamResponse) -> HubResponse {
    HubResponse {
        status_code: response.status,
        headers: headers_from(&response.headers),
        body: response.body,
    }
}
#[must_use]
pub fn mapping_headers(token: Option<&SecretString>) -> Headers {
    let mut headers = Headers::from([(String::from("Accept"), String::from("application/json"))]);
    if let Some(secret) = token {
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", secret.expose_secret()),
        );
    }
    headers
}
// GET https://huggingface.co/api/models/{model_id} on the dispatch's Hub client, with no status check
pub async fn model_info(
    deps: &DispatchDeps<'_>,
    model_id: &str,
    headers: &Headers,
    query: &[(&str, &str)],
) -> Result<HubResponse, HubDispatchError> {
    let url = format!("{HUB_URL}/api/models/{model_id}");
    let request = WireRequest {
        method: Method::GET,
        url: &url,
        query,
        headers,
        body: None,
        deadline: deps.timeout,
    };
    deps.hub.get(&request).await
}
pub struct PrefetchedHub(pub HubResponse);
impl HubGet for PrefetchedHub {
    fn get(
        &self,
        _url: &str,
        _headers: &Headers,
        _query: &[(&str, &str)],
    ) -> Result<HubResponse, TransportError> {
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
