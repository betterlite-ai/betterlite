use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use thiserror::Error;

use crate::image_edits::family;
use crate::openai_base_config::Headers;

use super::{resolved_api_base, resolved_api_key, versioned_url};

// PARAMS_VLLM_OMNI_DOES_NOT_ACCEPT
pub const UNSUPPORTED_PARAMS: [&str; 3] = ["input_fidelity", "mask", "quality"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageEditConfigError {
    #[error(
        "api_base not set for Hosted VLLM images edits API. Set via api_base parameter or HOSTED_VLLM_API_BASE environment variable"
    )]
    MissingApiBase,
}

// get_supported_openai_params is the OpenAI list minus the three vLLM-Omni refuses, in the same order
#[must_use]
pub fn get_supported_openai_params() -> Vec<&'static str> {
    family::SUPPORTED_OPENAI_PARAMS
        .iter()
        .copied()
        .filter(|name| !UNSUPPORTED_PARAMS.contains(name))
        .collect()
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
) -> Result<String, ImageEditConfigError> {
    let base =
        resolved_api_base(api_base, environment).ok_or(ImageEditConfigError::MissingApiBase)?;
    Ok(versioned_url(&base, "/images/edits"))
}
