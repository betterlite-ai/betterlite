use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::media::error::MediaError;

const DATA_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";

// The 50 MiB ceiling LiteLLM gives a downloaded image; an inline payload never needs more
pub const MAX_MEDIA_BYTES: usize = 52_428_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataUrl<'text> {
    pub media_type: &'text str,
    pub data: &'text str,
}

#[must_use]
pub fn parse_data_url(text: &str) -> Option<DataUrl<'_>> {
    let body = text.strip_prefix(DATA_URL_PREFIX)?;
    let (media_type, data) = body.split_once(BASE64_MARKER)?;
    Some(DataUrl { media_type, data })
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
        .count();
    payload
        .len()
        .saturating_div(4)
        .saturating_mul(3)
        .saturating_sub(padding)
}

// Python's b64decode(validate=True) followed by a canonical re-encoding, without allocating the bytes
#[must_use]
pub(crate) fn is_canonical_base64(payload: &str) -> bool {
    let bytes = payload.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }
    let data_length = bytes.len().saturating_sub(padding);
    if !bytes
        .iter()
        .take(data_length)
        .all(|byte| base64_value(*byte).is_some())
    {
        return false;
    }
    match padding {
        0 => true,
        1 => trailing_bits_are_zero(bytes, data_length, 0b11),
        2 => trailing_bits_are_zero(bytes, data_length, 0b1111),
        _ => false,
    }
}

fn trailing_bits_are_zero(bytes: &[u8], data_length: usize, mask: u8) -> bool {
    let Some(index) = data_length.checked_sub(1) else {
        return false;
    };
    let Some(byte) = bytes.get(index) else {
        return false;
    };
    base64_value(*byte).is_some_and(|value| value & mask == 0)
}

const fn base64_value(byte: u8) -> Option<u8> {
    if byte.is_ascii_uppercase() {
        return byte.checked_sub(b'A');
    }
    if byte.is_ascii_lowercase() {
        return match byte.checked_sub(b'a') {
            Some(value) => Some(value.saturating_add(26)),
            None => None,
        };
    }
    if byte.is_ascii_digit() {
        return match byte.checked_sub(b'0') {
            Some(value) => Some(value.saturating_add(52)),
            None => None,
        };
    }
    match byte {
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
