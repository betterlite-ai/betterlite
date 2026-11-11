use indexmap::IndexMap;
use serde::Deserialize;

use crate::controls::json_value::JsonValue;

pub const GENERATED_SESSION_METADATA_KEY: &str = "litellm_session_id_generated";

const FORBIDDEN_AFFINITY_HEADERS: [&str; 19] = [
    "api-key",
    "authorization",
    "connection",
    "content-length",
    "content-type",
    "cookie",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "set-cookie",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "www-authenticate",
    "x-api-key",
    "x-goog-api-key",
];

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AffinityParams {
    pub provider_affinity_header: Option<String>,
    pub session_id: Option<String>,
    pub litellm_session_id: Option<String>,
    pub metadata: Option<SessionMetadata>,
    pub litellm_metadata: Option<SessionMetadata>,
}

// The typed session view of a metadata object; every other key rides along untouched so the empty check and the logging copy stay exact.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct SessionMetadata {
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub litellm_session_id_generated: Option<bool>,
    #[serde(flatten)]
    pub other: IndexMap<String, JsonValue>,
}

impl SessionMetadata {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.session_id.is_none()
            && self.trace_id.is_none()
            && self.litellm_session_id_generated.is_none()
            && self.other.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AffinityError {
    #[error("provider_affinity_header must be a valid HTTP header name")]
    InvalidHeaderName,
    #[error("provider_affinity_header cannot be an authentication, cookie, or transport header")]
    ForbiddenHeader,
    #[error("session_id cannot contain HTTP header control characters")]
    ControlCharacters,
}

pub fn validate_affinity_header(header: &str) -> Result<&str, AffinityError> {
    if header.is_empty()
        || !header
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(AffinityError::InvalidHeaderName);
    }
    if FORBIDDEN_AFFINITY_HEADERS
        .iter()
        .any(|forbidden| header.eq_ignore_ascii_case(forbidden))
    {
        return Err(AffinityError::ForbiddenHeader);
    }
    Ok(header)
}

// A session id LiteLLM generated for logging is not a stable affinity key, so only ids that did not come from the generated marker reach the header.
#[must_use]
pub fn get_stable_session_id(params: &AffinityParams) -> Option<String> {
    if let Some(session_id) = params.session_id.as_deref() {
        return Some(session_id.to_owned());
    }
    let metadata_values = [params.metadata.as_ref(), params.litellm_metadata.as_ref()];
    let has_generated = metadata_values
        .iter()
        .flatten()
        .any(|metadata| metadata.litellm_session_id_generated == Some(true));
    if let Some(session_id) = params.litellm_session_id.as_deref()
        && !has_generated
    {
        return Some(session_id.to_owned());
    }
    for metadata in metadata_values.iter().flatten() {
        if metadata.litellm_session_id_generated != Some(true)
            && let Some(session_id) = metadata.session_id.as_deref()
        {
            return Some(session_id.to_owned());
        }
    }
    None
}

pub fn add_provider_affinity_header(
    headers: &IndexMap<String, String>,
    params: &AffinityParams,
) -> Result<IndexMap<String, String>, AffinityError> {
    let Some(configured) = params.provider_affinity_header.as_deref() else {
        return Ok(headers.clone());
    };
    let name = validate_affinity_header(configured)?;
    if headers
        .keys()
        .any(|existing| existing.eq_ignore_ascii_case(name))
    {
        return Ok(headers.clone());
    }
    let Some(session_id) = get_stable_session_id(params) else {
        return Ok(headers.clone());
    };
    if session_id
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | '\0'))
    {
        return Err(AffinityError::ControlCharacters);
    }
    let mut updated = headers.clone();
    updated.insert(name.to_owned(), session_id);
    Ok(updated)
}

mod tests;
