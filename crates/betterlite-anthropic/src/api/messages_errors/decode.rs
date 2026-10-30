use std::iter::Peekable;
use std::str::Bytes;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::wire::{ErrorRecord, ResponseRecord, TextField};

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum DecodedMessagesErrors {
    Error(Box<ErrorRecord>),
    Response(Box<ResponseRecord>),
}

#[derive(Debug, thiserror::Error)]
pub enum WireDecodeError {
    #[error("messages JSON exceeds the configured limit")]
    TooLarge,
    #[error("messages JSON has too many items")]
    TooManyItems,
    #[error("messages JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn decode_messages_errors(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<DecodedMessagesErrors, WireDecodeError> {
    let raw = bounded_json(bytes, limits)?;
    let response: ErrorRecord = serde_json::from_str(raw.get())?;
    match &response.head.r#type {
        TextField::Known(kind) if kind == "error" => {
            Ok(DecodedMessagesErrors::Error(Box::new(response)))
        }
        TextField::Missing | TextField::Other(_) | TextField::Known(_) => Ok(
            DecodedMessagesErrors::Response(Box::new(serde_json::from_str(raw.get())?)),
        ),
    }
}

pub(super) fn bounded_json(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<&RawValue, WireDecodeError> {
    check_bytes(bytes, limits)?;
    let raw: &RawValue = serde_json::from_slice(bytes)?;
    // Serde validates without an owned tree; count value tokens before allocating records.
    let mut tokens = raw.get().bytes().peekable();
    let mut remaining = limits.max_items;
    while let Some(token) = tokens.next() {
        if matches!(
            token,
            b' ' | b'\n' | b'\r' | b'\t' | b']' | b'}' | b',' | b':'
        ) {
            continue;
        }
        match token {
            b'"' => {
                skip_string(&mut tokens);
                while tokens.peek().is_some_and(u8::is_ascii_whitespace) {
                    tokens.next();
                }
                if tokens.peek() == Some(&b':') {
                    continue;
                }
            }
            b'{' | b'[' => {}
            _ => {
                while tokens.peek().is_some_and(|byte| {
                    !matches!(byte, b',' | b']' | b'}') && !byte.is_ascii_whitespace()
                }) {
                    tokens.next();
                }
            }
        }
        remaining = remaining
            .checked_sub(1)
            .ok_or(WireDecodeError::TooManyItems)?;
    }
    Ok(raw)
}

pub(super) const fn check_bytes(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<(), WireDecodeError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        Err(WireDecodeError::TooLarge)
    } else {
        Ok(())
    }
}

fn skip_string(tokens: &mut Peekable<Bytes<'_>>) {
    while let Some(character) = tokens.next() {
        match character {
            b'\\' => {
                tokens.next();
            }
            b'"' => break,
            _ => {}
        }
    }
}
