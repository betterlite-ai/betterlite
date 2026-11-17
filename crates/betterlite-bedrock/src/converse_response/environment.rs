use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::error::BedrockError as MetadataError;
use crate::request_policy::{BedrockRequestPolicy, Params, merge_invoke_headers};

// get_error_class hands the supplied status, message and headers to the provider error B682 owns; it fabricates no type, param or code.
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &[(String, String)],
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message: message.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    })
}

// A truthy api_key writes the canonical bearer header; no environment lookup happens here. The B596 merge then owns the metadata header.
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    api_key: Option<&SecretString>,
    policy: Option<&BedrockRequestPolicy>,
    litellm_params: &Params,
) -> Result<IndexMap<String, String>, MetadataError> {
    let mut merged_headers = headers.clone();
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        merged_headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    merge_invoke_headers(&merged_headers, &[], policy, litellm_params)
}

// Streaming selection only: an explicit fake_stream wins, then a streaming AI21 model, matched case-sensitively on the model name.
#[must_use]
pub fn should_fake_stream(
    model: Option<&str>,
    stream: Option<bool>,
    fake_stream: Option<bool>,
) -> bool {
    if fake_stream == Some(true) {
        return true;
    }
    stream == Some(true) && model.is_some_and(|name| name.contains("ai21"))
}
