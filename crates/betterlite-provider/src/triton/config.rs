use betterlite_prompts::builtin::Field;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;

use super::error::TritonError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["max_tokens", "max_completion_tokens"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TritonEndpoint {
    Generate,
    Infer,
}

impl TritonEndpoint {
    pub fn from_api_base(api_base: &str) -> Result<Self, TritonError> {
        if api_base.ends_with("/generate") {
            return Ok(Self::Generate);
        }
        if api_base.ends_with("/infer") {
            return Ok(Self::Infer);
        }
        Err(TritonError::InvalidApiBase {
            api_base: api_base.to_owned(),
        })
    }
}

pub fn complete_url(api_base: Option<&str>, stream: bool) -> Result<String, TritonError> {
    let Some(base) = api_base else {
        return Err(TritonError::MissingApiBase);
    };
    if TritonEndpoint::from_api_base(base)? == TritonEndpoint::Generate && stream {
        return Ok(format!("{base}_stream"));
    }
    Ok(base.to_owned())
}

pub fn map_openai_params(
    non_default_params: &IndexMap<String, Field>,
    optional_params: &mut IndexMap<String, Field>,
) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

#[must_use]
pub fn validate_environment() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}
