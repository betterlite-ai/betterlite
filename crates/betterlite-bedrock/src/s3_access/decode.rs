use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use super::{MANAGED_FILE_ID_MARKER, MANAGED_FILE_ID_PREFIX, S3_SCHEME, S3AccessError};

// Python's b64decode drops every byte outside the alphabet, takes trailing padding and keeps non-zero trailing bits
const DECODER: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

fn permissive_base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut symbols = Vec::with_capacity(input.len());
    let mut trailing_padding = 0_usize;
    for &byte in input {
        let symbol = match byte {
            b'-' => b'+',
            b'_' => b'/',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => byte,
            b'=' => {
                trailing_padding = trailing_padding.saturating_add(1);
                continue;
            }
            _ => continue,
        };
        symbols.push(symbol);
        trailing_padding = 0;
    }
    let enough_padding = match symbols.len() & 3 {
        0 => true,
        2 => trailing_padding >= 2,
        3 => trailing_padding >= 1,
        _ => false,
    };
    if !enough_padding {
        return None;
    }
    while !symbols.len().is_multiple_of(4) {
        symbols.push(b'=');
    }
    DECODER.decode(&symbols).ok()
}

fn decode_base64_text(file_id: &str) -> Option<String> {
    if !file_id.is_ascii() {
        return None;
    }
    // Python pads -len(file_id) % 4 characters before decoding, which also satisfies its trailing-padding rule
    let padding = 4_usize.wrapping_sub(file_id.len() & 3) & 3;
    let mut padded = String::with_capacity(file_id.len().saturating_add(padding));
    padded.push_str(file_id);
    for _ in 0..padding {
        padded.push('=');
    }
    let decoded = permissive_base64_decode(padded.as_bytes())?;
    String::from_utf8(decoded).ok()
}

fn decode_managed_text(file_id: &str) -> Option<String> {
    let decoded = decode_base64_text(file_id)?;
    if !decoded.starts_with(MANAGED_FILE_ID_PREFIX) {
        return None;
    }
    let rest = decoded.split_once(MANAGED_FILE_ID_MARKER)?.1;
    let marker_at = rest.find(MANAGED_FILE_ID_MARKER).unwrap_or(rest.len());
    let semicolon_at = rest.find(';').unwrap_or(rest.len());
    rest.get(..marker_at.min(semicolon_at)).map(str::to_owned)
}

pub fn decode_managed_file_id(file_id: &str) -> Result<String, S3AccessError> {
    if let Some(decoded) = decode_managed_text(file_id) {
        return Ok(decoded);
    }
    if file_id.starts_with(S3_SCHEME) {
        return Ok(file_id.to_owned());
    }
    Err(S3AccessError::ManagedFileId)
}
