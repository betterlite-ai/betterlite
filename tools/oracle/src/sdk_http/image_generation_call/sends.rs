use std::time::Duration;

use betterlite_core::api::image_generation_call::error::provider_failure;
use betterlite_core::api::image_generation_call::operations::{BranchAnswer, Headers};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageResponse, ImageUsage};
use betterlite_openai::image_generation::transport::ImageGenerationTransport as _;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use url::Url;

pub(super) mod custom;
pub(super) mod foundry;
pub(super) mod generic;
pub(super) mod openai;
pub(super) mod providers;

pub(super) fn plain(message: &str, model: &str, provider: &str) -> Box<RawException> {
    Box::new(RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    })
}

pub(super) struct Post<'post> {
    pub(super) url: String,
    pub(super) headers: &'post Headers,
    pub(super) body: Json,
    pub(super) timeout: Duration,
    pub(super) model: &'post str,
    pub(super) provider: &'post str,
    pub(super) exception_type: &'post str,
}

async fn post(
    client: &ProxiedClient,
    request: Post<'_>,
) -> Result<UpstreamResponse, Box<RawException>> {
    let url = Url::parse(&request.url)
        .map_err(|error| plain(&error.to_string(), request.model, request.provider))?;
    let mut headers = HeaderMap::new();
    for (name, value) in request.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| plain(&error.to_string(), request.model, request.provider))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| plain(&error.to_string(), request.model, request.provider))?;
        headers.insert(header_name, header_value);
    }
    let body = serde_json::to_vec(&request.body)
        .map_err(|error| plain(&error.to_string(), request.model, request.provider))?;
    let outbound = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline: request.timeout,
    };
    let response = client
        .generate(outbound)
        .await
        .map_err(|error| plain(&error.to_string(), request.model, request.provider))?;
    if !(200..300).contains(&response.status) {
        let text = String::from_utf8(response.body.clone()).unwrap_or_default();
        return Err(Box::new(provider_failure(
            response.status,
            &text,
            request.model,
            request.provider,
            request.exception_type,
        )));
    }
    Ok(response)
}

// LiteLLM's azure_deployment_image_generation_json_body drops the model key on this deployment-scoped URL
pub(super) fn prompt_body(prompt: &str, optional_params: &IndexMap<String, Json>) -> Json {
    let mut body = IndexMap::new();
    body.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
    body.extend(optional_params.clone());
    Json::Object(body)
}

pub(super) fn model_prompt_body(
    model: &str,
    prompt: &str,
    optional_params: &IndexMap<String, Json>,
) -> Json {
    let mut body = IndexMap::new();
    body.insert(String::from("prompt"), Json::Text(prompt.to_owned()));
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.extend(optional_params.clone());
    Json::Object(body)
}

fn parsed_response(
    bytes: &[u8],
    model: &str,
    provider: &str,
) -> Result<ImageResponse, Box<RawException>> {
    let mut response: ImageResponse = serde_json::from_slice(bytes)
        .map_err(|error| plain(&error.to_string(), model, provider))?;
    // LiteLLM's ImageResponse starts with a zero usage, and the provider transforms leave it there
    if response.usage.is_none() {
        response.usage = Some(ImageUsage::default());
    }
    Ok(response)
}

// The provider answer every send projects: its body and the hidden model it came from
pub(super) async fn answer(
    client: &ProxiedClient,
    request: Post<'_>,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model;
    let provider = request.provider;
    let response = post(client, request).await?;
    Ok(BranchAnswer {
        response: parsed_response(&response.body, model, provider)?,
        hidden_model: None,
    })
}

pub(super) fn inserts_bearer(headers: &mut Headers, token: &SecretString) {
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", token.expose_secret()),
    );
}

pub(super) fn has_auth(headers: &Headers) -> bool {
    headers.keys().any(|name| {
        name.eq_ignore_ascii_case("api-key") || name.eq_ignore_ascii_case("authorization")
    })
}
