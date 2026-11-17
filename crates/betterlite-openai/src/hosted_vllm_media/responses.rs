use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;
use thiserror::Error;

use crate::openai_base_config::Headers;

use super::{resolved_api_base, resolved_api_key, versioned_url};

// LiteLLM's HostedVLLMResponsesAPIConfig overrides only the provider, the key and the URL
pub const PROVIDER: ProviderId = ProviderId::HostedVllm;
// supports_native_websocket returns False, so the OpenAI transport stays the one used
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResponsesConfigError {
    #[error(
        "api_base not set for Hosted VLLM responses API. Set via api_base parameter or HOSTED_VLLM_API_BASE environment variable"
    )]
    MissingApiBase,
}

// validate_environment overwrites Authorization with the resolved key, which is the vLLM placeholder when none is set
#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", resolved_api_key(api_key, environment)),
    );
    headers
}

pub fn get_complete_url(
    api_base: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Result<String, ResponsesConfigError> {
    let base =
        resolved_api_base(api_base, environment).ok_or(ResponsesConfigError::MissingApiBase)?;
    Ok(versioned_url(&base, "/responses"))
}
