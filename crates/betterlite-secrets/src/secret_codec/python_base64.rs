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

// Python 3.13's non-validating decoder skips padding and nonalphabet bytes before decoding.
pub(crate) fn decode_lenient(text: &str, url_alphabet: bool) -> Result<Vec<u8>, SecretError> {
    bounded_ascii(text)?;
    let mut sextets = String::with_capacity(text.len());
    for byte in text.bytes() {
        let normalized = match (byte, url_alphabet) {
            (b'-', true) => '+',
            (b'_', true) => '/',
            (b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/', _) => char::from(byte),
            _ => continue,
        };
        sextets.push(normalized);
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
