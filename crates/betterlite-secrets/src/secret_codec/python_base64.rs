use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

use crate::error::SecretError;
use crate::secret_codec::MAX_ENCODED_BYTES;

const SYMBOLS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

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

// CPython keeps the stray low bits of a final partial group and the base64 crate refuses them
fn clear_stray_bits(encoded: &mut String) {
    let data = encoded.trim_end_matches('=').len();
    let mask = match data & 3 {
        2 => 0b11_0000,
        3 => 0b11_1100,
        _ => return,
    };
    let Some(last) = data.checked_sub(1) else {
        return;
    };
    let index = encoded
        .get(last..data)
        .and_then(|symbol| SYMBOLS.find(symbol));
    let cleared = index.and_then(|value| SYMBOLS.get(value & mask..=value & mask));
    if let Some(symbol) = cleared {
        encoded.replace_range(last..data, symbol);
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
                (Group::Two, true) | (Group::Three, _) => return finish(sextets, true),
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
    finish(sextets, matches!(group, Group::Empty))
}

fn finish(mut sextets: String, complete: bool) -> Result<Vec<u8>, SecretError> {
    if !complete {
        return Err(SecretError::Base64);
    }
    clear_stray_bits(&mut sextets);
    STANDARD_NO_PAD
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
    clear_stray_bits(&mut padded);
    STANDARD
        .decode(padded)
        .map_err(|source| SecretError::Decode { source })
}
