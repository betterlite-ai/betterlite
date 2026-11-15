use std::str::from_utf8;

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::{
    McpStoredDecodeError, decode_mcp_stored_contract,
};
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::MAX_ENCODED_BYTES;

use super::error::RotationCodecError;

// This preserves CPython's acceptance of stray low bits in the final partial group.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

pub fn decode_mcp_rotation_blob(bytes: &[u8]) -> Result<JsonValue, RotationCodecError> {
    let text = from_utf8(bytes).map_err(RotationCodecError::Utf8)?;
    match decode_mcp_stored_contract(bytes) {
        Ok(value) => Ok(value),
        Err(McpStoredDecodeError::Json(_)) => {
            decode_legacy_base64_text(text).map(JsonValue::String)
        }
        Err(error) => Err(RotationCodecError::Blob(error)),
    }
}

pub fn decode_legacy_base64_text(text: &str) -> Result<String, RotationCodecError> {
    if text.len() > MAX_ENCODED_BYTES {
        return Err(RotationCodecError::Secret(SecretError::TooLarge {
            length: text.len(),
            limit: MAX_ENCODED_BYTES,
        }));
    }
    if !text.is_ascii() {
        return Err(RotationCodecError::Secret(SecretError::Base64));
    }
    let mut sextets = String::with_capacity(text.len());
    let mut trailing_pads = 0_usize;
    for byte in text.bytes() {
        let normalized = match byte {
            b'-' => '+',
            b'_' => '/',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => char::from(byte),
            b'=' => {
                trailing_pads = trailing_pads.saturating_add(1);
                continue;
            }
            _ => continue,
        };
        trailing_pads = 0;
        sextets.push(normalized);
    }
    let remainder = sextets.len() & 3;
    if matches!(remainder, 2 | 3) && remainder.saturating_add(trailing_pads) < 4 {
        return Err(RotationCodecError::Secret(SecretError::Base64));
    }
    let decoded = LENIENT
        .decode(sextets)
        .map_err(|source| RotationCodecError::Secret(SecretError::Decode { source }))?;
    String::from_utf8(decoded).map_err(|error| RotationCodecError::Utf8(error.utf8_error()))
}
