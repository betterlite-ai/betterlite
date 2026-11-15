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
    let mut headers = validate_environment(
        HeaderMap::new(),
        call.params.api_key.as_ref(),
        call.configured_key,
    )?;
    // LiteLLM overlays the caller's headers after credential validation, so the caller can replace x-key
    for (name, value) in call.extra_headers.unwrap_or_default() {
        overlay_header(&mut headers, name, value)?;
    }
    let url = complete_url(
        call.params.api_base.as_deref(),
        call.model,
        call.configured_base,
    )?;
    let body = request_body(call.prompt, call.optional_params);
    Ok(PreparedGeneration { url, headers, body })
}

fn overlay_header(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
) -> Result<(), GenerationCallError> {
    let header = HeaderName::try_from(name).map_err(|source| GenerationCallError::HeaderName {
        name: name.to_owned(),
        source,
    })?;
    let shown = header.as_str().to_owned();
    let value =
        HeaderValue::try_from(value).map_err(|source| GenerationCallError::HeaderValue {
            name: shown,
            source,
        })?;
    headers.insert(header, value);
    Ok(())
}
