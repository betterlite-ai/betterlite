use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::hosted_vllm::embedding::error::HostedVllmEmbeddingError;
use betterlite_openai::hosted_vllm::embedding::{
    get_complete_url, transform_embedding_request, transform_embedding_response,
    validate_environment,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::invalid;
use betterlite_oracle_helper_api::error::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersCall {
    headers: Headers,
    api_key: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    api_base: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    model: String,
    input: EmbeddingInput,
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    body: String,
}

pub(super) fn headers_value(call: &HeadersCall) -> Result<Value, Box<HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let headers = validate_environment(&call.headers, key.as_ref(), &call.env);
    serde_json::to_value(headers).map_err(|error| invalid(error.to_string()))
}

pub(super) fn url_value(call: &UrlCall) -> Result<Value, Box<HelperError>> {
    get_complete_url(call.api_base.as_deref(), &call.env)
        .map(Value::String)
        .map_err(|error| invalid(error.to_string()))
}

pub(super) fn request_value(call: &RequestCall) -> Value {
    Value::Object(transform_embedding_request(
        &call.model,
        &call.input,
        &call.optional_params,
    ))
}

pub(super) fn response_value(call: &ResponseCall) -> Result<Value, Box<HelperError>> {
    let response = transform_embedding_response(call.body.as_bytes(), &mut |_text| {})
        .map_err(response_failure)?;
    let data = serde_json::to_value(response.data).map_err(|error| invalid(error.to_string()))?;
    let usage = response.usage;
    Ok(json!({
        "model": response.model,
        "object": response.object,
        "data": data,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }))
}

// LiteLLM's error in a 2xx reply carries its own status, so the oracle sees the same one
fn response_failure(error: HostedVllmEmbeddingError) -> Box<HelperError> {
    match error {
        HostedVllmEmbeddingError::Provider {
            message,
            status_code,
            ..
        } => Box::new(HelperError {
            class: String::from("ValueError"),
            status: Some(status_code),
            error_type: None,
            param: None,
            code: None,
            message,
        }),
        other @ (HostedVllmEmbeddingError::MissingApiBase
        | HostedVllmEmbeddingError::Shape { .. }) => invalid(other.to_string()),
    }
}
