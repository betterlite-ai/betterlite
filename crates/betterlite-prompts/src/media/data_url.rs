use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::media::error::MediaError;

const DATA_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";
// Only two padding characters can belong to a payload; more is invalid and must not shrink the size count
const TRAILING_PADDING: usize = 2;

// The 50 MiB ceiling LiteLLM gives a downloaded image; an inline payload never needs more
pub const MAX_MEDIA_BYTES: usize = 52_428_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataUrl<'text> {
    pub media_type: &'text str,
    pub data: &'text str,
}

// LiteLLM strips the prefix when it is there and splits anyway, so a bare payload also parses
#[must_use]
pub fn split_base64_marker(text: &str) -> Option<DataUrl<'_>> {
    let body = text.strip_prefix(DATA_URL_PREFIX).unwrap_or(text);
    let (media_type, data) = body.split_once(BASE64_MARKER)?;
    Some(DataUrl { media_type, data })
}

pub fn decode_data_url(text: &str, max_bytes: usize) -> Result<Vec<u8>, MediaError> {
    let Some(body) = text.strip_prefix(DATA_URL_PREFIX) else {
        return Err(MediaError::MissingDataPrefix);
    };
    let Some((_media_type, data)) = body.split_once(BASE64_MARKER) else {
        return Err(MediaError::MissingBase64Marker);
    };
    if decoded_length(data) > max_bytes {
        return Err(MediaError::TooLarge { limit: max_bytes });
    }
    Ok(STANDARD.decode(data)?)
}

// The decoded size is counted from the payload text before the decoder allocates the bytes
fn decoded_length(payload: &str) -> usize {
    let padding = payload
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .take(TRAILING_PADDING)
        .count();
    payload
        .len()
        .saturating_div(4)
        .saturating_mul(3)
        .saturating_sub(padding)
}

// STANDARD rejects non-alphabet bytes, non-canonical padding and non-zero trailing bits, which is Python's validate=True decode plus a re-encoding comparison
#[must_use]
pub(crate) fn is_canonical_base64(payload: &str, max_bytes: usize) -> bool {
    if decoded_length(payload) > max_bytes {
        return false;
    }
    STANDARD.decode(payload).is_ok()
}
