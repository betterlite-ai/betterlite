use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use super::TypedValue;
use crate::resolved_key::credential::is_jwt;

pub const MAX_CLAIM_BYTES: usize = 0x0010_0000;
pub const MAX_CLAIM_DEPTH: usize = 64;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the compact JWT payload is {bytes} bytes, over the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the compact JWT payload nests deeper than {limit} JSON levels")]
    TooDeep { limit: usize },
    #[error("the JWT segment is not canonical base64url")]
    Base64(#[source] base64::DecodeError),
    #[error("the JWT segment has invalid padding")]
    Padding,
    #[error("the JWT segment is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the JWT payload is not a JSON object")]
    NotAnObject,
    #[error("the JWT payload could not be rebuilt as a typed document")]
    Document(#[source] serde_json::Error),
}

// the compact JWT payload segment, before any trust decision; pyjwt's segment decoder rejects the same shapes
pub fn decode_unverified_claims(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_CLAIM_BYTES {
        return Err(DecodeError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_CLAIM_BYTES,
        });
    }
    let payload = decode_segment(bytes)?;
    let tree: JsonValue = serde_json::from_slice(&payload).map_err(DecodeError::Json)?;
    if depth_exceeds(&tree, MAX_CLAIM_DEPTH) {
        return Err(DecodeError::TooDeep {
            limit: MAX_CLAIM_DEPTH,
        });
    }
    if !matches!(&tree, JsonValue::Object(_)) {
        return Err(DecodeError::NotAnObject);
    }
    let json = serde_json::to_value(&tree).map_err(DecodeError::Document)?;
    Ok(TypedValue { json })
}

// JWTHandler.get_unverified_claims: three segments, a usable header and signature, then the payload object
#[must_use]
pub fn unverified_claims(token: &str) -> Option<TypedValue> {
    if !is_jwt(Some(token)) {
        return None;
    }
    let mut segments = token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let signature = segments.next()?;
    if !header_is_valid(header.as_bytes()) {
        return None;
    }
    decode_segment(signature.as_bytes()).ok()?;
    decode_unverified_claims(payload.as_bytes()).ok()
}

// pyjwt strips up to two trailing '=', requires a padded segment to be a whole quad and re-encodes the result
fn decode_segment(segment: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let padding = segment
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if padding > 2 {
        return Err(DecodeError::Padding);
    }
    if padding > 0 && !segment.len().is_multiple_of(4) {
        return Err(DecodeError::Padding);
    }
    let end = segment.len().saturating_sub(padding);
    let symbols = segment.get(..end).ok_or(DecodeError::Padding)?;
    if symbols.len() & 3 == 1 {
        return Err(DecodeError::Padding);
    }
    URL_SAFE_NO_PAD.decode(symbols).map_err(DecodeError::Base64)
}

// pyjwt parses the header first and validates its kid, crit and b64 parameters, and a detached b64 payload never decodes here
fn header_is_valid(segment: &[u8]) -> bool {
    let Ok(bytes) = decode_segment(segment) else {
        return false;
    };
    let Ok(JsonValue::Object(fields)) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return false;
    };
    if matches!(fields.get("b64"), Some(JsonValue::Bool(false))) {
        return false;
    }
    if let Some(kid) = fields.get("kid")
        && !matches!(kid, JsonValue::Text(_))
    {
        return false;
    }
    let Some(crit) = fields.get("crit") else {
        return true;
    };
    let JsonValue::Array(extensions) = crit else {
        return false;
    };
    if extensions.is_empty() {
        return false;
    }
    for extension in extensions {
        let JsonValue::Text(name) = extension else {
            return false;
        };
        if name != "b64" || !fields.contains_key(name.as_str()) {
            return false;
        }
    }
    true
}

fn depth_exceeds(value: &JsonValue, limit: usize) -> bool {
    let mut pending = vec![(value, 0_usize)];
    while let Some((current, level)) = pending.pop() {
        match current {
            JsonValue::Array(items) => {
                let nested = level.saturating_add(1);
                if nested > limit {
                    return true;
                }
                pending.extend(items.iter().map(|item| (item, nested)));
            }
            JsonValue::Object(fields) => {
                let nested = level.saturating_add(1);
                if nested > limit {
                    return true;
                }
                pending.extend(fields.values().map(|field| (field, nested)));
            }
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {}
        }
    }
    false
}
