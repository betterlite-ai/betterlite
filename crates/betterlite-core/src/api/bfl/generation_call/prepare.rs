use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::GenerationCall;
use super::error::GenerationCallError;
use crate::api::bfl::generation::{complete_url, request_body, validate_environment};

pub struct PreparedGeneration {
    pub url: String,
    pub headers: HeaderMap,
    pub body: IndexMap<String, Json>,
}

pub fn prepare(call: &GenerationCall<'_>) -> Result<PreparedGeneration, GenerationCallError> {
    let headers = validate_environment(
        HeaderMap::new(),
        call.params.api_key.as_ref(),
        call.configured_key,
    )?;
    let url = complete_url(
        call.params.api_base.as_deref(),
        call.model,
        call.configured_base,
    )?;
    let body = request_body(call.prompt, call.optional_params);
    Ok(PreparedGeneration { url, headers, body })
}

// LiteLLM overlays the caller's headers after credential validation, so the caller can replace x-key; a header httpx cannot encode only fails at the POST, after the pre-call record
pub fn overlay_headers(
    validated: &HeaderMap,
    extra_headers: &[(String, String)],
) -> Result<HeaderMap, GenerationCallError> {
    let mut headers = validated.clone();
    for (name, value) in extra_headers {
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            GenerationCallError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        // httpx encodes header values as ASCII, which a HeaderValue would not enforce
        if !value.is_ascii() {
            return Err(GenerationCallError::HeaderNotAscii {
                name: header_name.as_str().to_owned(),
            });
        }
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            GenerationCallError::HeaderValue {
                name: header_name.as_str().to_owned(),
                source,
            }
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}
