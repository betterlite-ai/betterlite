use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use super::config::Params;
use super::guardrail::{GuardrailError, guardrail_headers};
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::error::BedrockError as MetadataError;
use crate::request_policy::{BedrockRequestPolicy, Params as MetadataParams, merge_invoke_headers};

#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailError),
    #[error("guardrailConfig cannot be decoded")]
    GuardrailJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Headers(#[from] HeaderError),
}

// LiteLLM takes an api_key here and never turns it into an Authorization header
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    optional_params: &mut Params,
    litellm_params: &MetadataParams,
    policy: Option<&BedrockRequestPolicy>,
) -> Result<IndexMap<String, String>, EnvironmentError> {
    let guardrail = match optional_params.shift_remove("guardrailConfig") {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str::<Option<Json>>(raw.get()) {
            Ok(None) => Vec::new(),
            Ok(Some(_)) => guardrail_headers(&raw)?,
            Err(source) => return Err(EnvironmentError::GuardrailJson { source }),
        },
    };
    merge_invoke_headers(headers, &guardrail, policy, litellm_params)
        .map_err(EnvironmentError::from)
}

pub fn error_class(
    message: String,
    status_code: u16,
    headers: Vec<(String, String)>,
) -> Result<BedrockError, EnvironmentError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers)),
        response: None,
    })
    .map_err(EnvironmentError::from)
}
