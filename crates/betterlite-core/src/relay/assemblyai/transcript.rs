use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use indexmap::IndexMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

use super::AssemblyAiError;

// urllib.parse.quote(id, safe=""): every byte outside the unreserved set is escaped
const TRANSCRIPT_ID_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const DISALLOWED_ID_FRAGMENTS: [char; 4] = ['/', '\\', '#', '?'];

// A null transcript field reads as absent, and any other non-text value names the field
pub(super) fn text_field<'body>(
    fields: &'body IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'body str>, AssemblyAiError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(AssemblyAiError::TextField { field }),
    }
}

// The transcript ID lands in a URL path, so the characters a path cannot carry safely are rejected
pub(super) fn encode_transcript_id(transcript_id: &str) -> Result<String, AssemblyAiError> {
    let rejected = DISALLOWED_ID_FRAGMENTS
        .iter()
        .any(|fragment| transcript_id.contains(*fragment))
        || transcript_id.contains("..");
    if rejected {
        return Err(AssemblyAiError::InvalidTranscriptId {
            transcript_id: transcript_id.to_owned(),
        });
    }
    Ok(utf8_percent_encode(transcript_id, TRANSCRIPT_ID_ENCODE_SET).to_string())
}

// An unbuildable header fails the request the way LiteLLM's caught httpx error does
pub(super) fn request_headers(api_key: &SecretString) -> Option<HeaderMap> {
    let mut headers = HeaderMap::new();
    let bearer = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret())).ok()?;
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Some(headers)
}
