use base64::DecodeError;
use base64::Engine as _;
use base64::engine::GeneralPurpose;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use data_encoding::BASE64_MIME_PERMISSIVE;

use crate::error::SecretError;
use crate::secret_codec::MAX_ENCODED_BYTES;

#[derive(Clone, Copy)]
enum Group {
    Empty,
    One,
    Two,
    Three,
}

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

// Python's non-validating b64decode: skips bytes outside the alphabet and stops at the first full pad
pub(crate) fn decode_lenient(text: &str, url_alphabet: bool) -> Result<Vec<u8>, SecretError> {
    bounded_ascii(text)?;
    let mut sextets = String::with_capacity(text.len());
    let mut group = Group::Empty;
    let mut half_padded = false;
    for byte in text.bytes() {
        let normalized = match (byte, url_alphabet) {
            (b'=', _) => match (group, half_padded) {
                (Group::Two, false) => {
                    half_padded = true;
                    continue;
                }
                (Group::Two, true) | (Group::Three, _) => return finish(&sextets, true),
                (Group::Empty | Group::One, _) => continue,
            },
            (b'-', true) => '+',
            (b'_', true) => '/',
            (b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/', _) => char::from(byte),
            _ => continue,
        };
        half_padded = false;
        sextets.push(normalized);
        group = match group {
            Group::Empty => Group::One,
            Group::One => Group::Two,
            Group::Two => Group::Three,
            Group::Three => Group::Empty,
        };
    }
    let complete = match group {
        Group::Empty => true,
        Group::One | Group::Two | Group::Three => false,
    };
    finish(&sextets, complete)
}

fn finish(sextets: &str, complete: bool) -> Result<Vec<u8>, SecretError> {
    if !complete {
        return Err(SecretError::Base64);
    }
    decode(&STANDARD_NO_PAD, sextets)
}

fn decode(engine: &GeneralPurpose, encoded: &str) -> Result<Vec<u8>, SecretError> {
    match engine.decode(encoded) {
        // Only relax trailing bits after base64 has validated the alphabet, length and padding.
        Err(DecodeError::InvalidLastSymbol { .. }) => {
            let mut padded = encoded.to_owned();
            while !padded.len().is_multiple_of(4) {
                padded.push('=');
            }
            BASE64_MIME_PERMISSIVE
                .decode(padded.as_bytes())
                .map_err(|source| SecretError::DecodeTail { source })
        }
        outcome => outcome.map_err(|source| SecretError::Decode { source }),
    }
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
    decode(&STANDARD, &padded)
}
