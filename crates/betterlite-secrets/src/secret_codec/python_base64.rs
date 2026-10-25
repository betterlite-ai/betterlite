use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use crate::error::SecretError;
use crate::secret_codec::MAX_ENCODED_BYTES;

const fn bounded_ascii(text: &str) -> Result<(), SecretError> {
    if text.len() > MAX_ENCODED_BYTES {
        return Err(SecretError::TooLarge {
            length: text.len(),
            limit: MAX_ENCODED_BYTES,
        });
    }
    if text.is_ascii() {
        Ok(())
    } else {
        Err(SecretError::Base64)
    }
}

// These engines preserve CPython's acceptance of stray low bits in final partial groups.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);
const STRICT: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

// Python 3.13 ignores interior padding but requires padding for a final partial group.
pub(crate) fn decode_lenient(text: &str, url_alphabet: bool) -> Result<Vec<u8>, SecretError> {
    bounded_ascii(text)?;
    let mut sextets = String::with_capacity(text.len());
    let mut trailing_pads = 0_usize;
    for byte in text.bytes() {
        let normalized = match (byte, url_alphabet) {
            (b'-', true) => '+',
            (b'_', true) => '/',
            (b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/', _) => char::from(byte),
            (b'=', _) => {
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
        return Err(SecretError::Base64);
    }
    LENIENT
        .decode(sextets)
        .map_err(|source| SecretError::Decode { source })
}

// Python's validating b64decode with altchars "-_": both alphabets pass, padding must be canonical
pub(crate) fn decode_strict(encoded: &str) -> Result<Vec<u8>, SecretError> {
    bounded_ascii(encoded)?;
    let mut padded: String = encoded
        .chars()
        .map(|symbol| match symbol {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    STRICT
        .decode(padded)
        .map_err(|source| SecretError::Decode { source })
}
