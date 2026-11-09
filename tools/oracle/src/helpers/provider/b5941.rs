use std::collections::BTreeMap;

use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_provider::oci_common::OciError;
use betterlite_provider::oci_common::credentials::{
    OciEnvValues, OciExplicitParams, resolve_oci_credentials,
};
use betterlite_provider::oci_common::environment::OciEnvironment;
use betterlite_provider::oci_embed::response::decode_embed_response;
use betterlite_provider::oci_embed::{
    OciEmbedParams, SUPPORTED_OPENAI_PARAMS, get_complete_url, map_openai_params,
    transform_embedding_request, validate_environment,
};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
enum Step {
    SupportedParams(SupportedStep),
    MapParams(Box<MapStep>),
    ValidateEnvironment(Box<ValidateStep>),
    CompleteUrl(Box<UrlStep>),
    Request(Box<RequestStep>),
    Response(ResponseStep),
}

#[derive(Debug, Deserialize)]
struct SupportedStep {
    model: String,
}

#[derive(Debug, Deserialize)]
struct MapStep {
    #[serde(default)]
    non_default_params: OciEmbedParams,
    #[serde(default)]
    optional_params: OciEmbedParams,
}

#[derive(Debug, Deserialize)]
struct ValidateStep {
    #[serde(default)]
    explicit: OciExplicitParams,
    #[serde(default)]
    env: OciEnvValues,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct UrlStep {
    #[serde(default)]
    explicit: OciExplicitParams,
    #[serde(default)]
    env: OciEnvValues,
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RequestStep {
    #[serde(default)]
    explicit: OciExplicitParams,
    #[serde(default)]
    env: OciEnvValues,
    model: String,
    input: EmbeddingInput,
    #[serde(default)]
    non_default_params: OciEmbedParams,
    #[serde(default)]
    optional_params: OciEmbedParams,
}

#[derive(Debug, Deserialize)]
struct ResponseStep {
    status: u16,
    body: String,
}

fn encode<T>(value: T) -> Result<serde_json::Value, OciError>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|source| OciError {
        status: 500,
        message: format!("the helper value could not be encoded: {source}"),
    })
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, OciError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| OciError {
            status: 500,
            message: format!("the helper header name {name} is not valid: {source}"),
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|source| OciError {
            status: 500,
            message: format!("the helper header value for {name} is not valid: {source}"),
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn headers_json(headers: &HeaderMap) -> Result<serde_json::Value, OciError> {
    let mut rendered = BTreeMap::new();
    for (name, value) in headers {
        let text = value.to_str().map_err(|source| OciError {
            status: 500,
            message: format!("the helper header value is not text: {source}"),
        })?;
        rendered.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(serde_json::json!(rendered))
}

fn project_response(response: &EmbeddingResponse) -> serde_json::Value {
    serde_json::json!({
        "model": response.model,
        "object": response.object,
        "data": response.data,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn supported_output(step: &SupportedStep) -> serde_json::Value {
    serde_json::json!({
        "model": step.model,
        "params": SUPPORTED_OPENAI_PARAMS,
    })
}

fn map_output(step: &MapStep) -> Result<serde_json::Value, OciError> {
    encode(map_openai_params(
        &step.non_default_params,
        step.optional_params.clone(),
    ))
}

fn validate_output(step: &ValidateStep) -> Result<serde_json::Value, OciError> {
    let credentials = resolve_oci_credentials(&step.explicit, &step.env);
    let mut request_headers = header_map(&step.headers)?;
    validate_environment(
        &mut request_headers,
        &credentials,
        &OciEnvironment::default(),
    )?;
    headers_json(&request_headers)
}

fn url_output(step: &UrlStep) -> Result<serde_json::Value, OciError> {
    let credentials = resolve_oci_credentials(&step.explicit, &step.env);
    Ok(serde_json::json!(get_complete_url(
        &credentials,
        step.api_base.as_deref()
    )?))
}

fn request_output(step: &RequestStep) -> Result<serde_json::Value, OciError> {
    let credentials = resolve_oci_credentials(&step.explicit, &step.env);
    let mapped = map_openai_params(&step.non_default_params, step.optional_params.clone());
    let body = transform_embedding_request(&step.model, &step.input, &mapped, &credentials)?;
    encode(body)
}

fn response_output(step: &ResponseStep) -> Result<serde_json::Value, OciError> {
    Ok(project_response(&decode_embed_response(
        step.status,
        step.body.as_bytes(),
    )?))
}

fn step_result(item: &Step) -> Result<serde_json::Value, OciError> {
    match item {
        Step::SupportedParams(step) => Ok(supported_output(step)),
        Step::MapParams(step) => map_output(step),
        Step::ValidateEnvironment(step) => validate_output(step),
        Step::CompleteUrl(step) => url_output(step),
        Step::Request(step) => request_output(step),
        Step::Response(step) => response_output(step),
    }
}

fn step_output(step: &Step) -> serde_json::Value {
    match step_result(step) {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err(error) => serde_json::json!({"ok": false, "status": error.status}),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let results = input.steps.iter().map(step_output).collect::<Vec<_>>();
        Ok(serde_json::json!(results))
    }
}
