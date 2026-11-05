use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::{DecodeError, Engine};
use serde::Deserialize;
use thiserror::Error;

// Python's b64decode accepts non-canonical trailing bits.
const LENIENT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the JSON file payload is invalid: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the base64 file content is invalid: {0}")]
    Base64(#[source] DecodeError),
}

#[derive(Deserialize)]
struct GitLabFilePayload {
    content: Option<String>,
    encoding: Option<String>,
}

#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

pub fn decode_file_payload(body: &[u8]) -> Result<Option<String>, PayloadError> {
    let payload: GitLabFilePayload = serde_json::from_slice(body).map_err(PayloadError::Json)?;
    let Some(content) = payload.content else {
        return Ok(None);
    };
    if content.is_empty() || payload.encoding.as_deref() != Some("base64") {
        return Ok(Some(content));
    }
    // Python's b64decode drops every character outside the alphabet before decoding.
    let alphabet_only: String = content
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "+/=".contains(*character))
        .collect();
    let decoded = LENIENT_BASE64
        .decode(alphabet_only)
        .map_err(PayloadError::Base64)?;
    Ok(Some(decode_text(&decoded)))
}

mod tests;
