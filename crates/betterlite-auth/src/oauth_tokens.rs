use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

pub mod check;
pub mod extract;
pub mod proxy_identity;
pub mod request;
pub mod transport;

mod tests;

pub const MAX_OAUTH_RESPONSE_BYTES: usize = 0x0010_0000;
pub const MAX_OAUTH_RESPONSE_DEPTH: usize = 64;

pub type TypedValue = JsonValue;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the OAuth token response is {bytes} bytes, over the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the OAuth token response nests {depth} levels, over the {limit}-level limit")]
    TooDeep { depth: usize, limit: usize },
    #[error("the OAuth token response is not valid JSON")]
    Json(#[source] serde_json::Error),
}

// Outside bytes stop here: both bounds run before any policy function sees the document.
pub fn decode_oauth_response(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_OAUTH_RESPONSE_BYTES {
        return Err(DecodeError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_OAUTH_RESPONSE_BYTES,
        });
    }
    let document: TypedValue = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    let depth = nesting_depth(&document);
    if depth > MAX_OAUTH_RESPONSE_DEPTH {
        return Err(DecodeError::TooDeep {
            depth,
            limit: MAX_OAUTH_RESPONSE_DEPTH,
        });
    }
    Ok(document)
}

// A top-level object is one level; serde_json's own recursion limit stays below its stack bound
fn nesting_depth(document: &TypedValue) -> usize {
    let mut pending = vec![(document, 0_usize)];
    let mut deepest = 0_usize;
    while let Some((current, level)) = pending.pop() {
        let nested = match current {
            JsonValue::Array(items) => {
                pending.extend(items.iter().map(|item| (item, level.saturating_add(1))));
                level.saturating_add(1)
            }
            JsonValue::Object(fields) => {
                pending.extend(
                    fields
                        .values()
                        .map(|field| (field, level.saturating_add(1))),
                );
                level.saturating_add(1)
            }
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {
                continue;
            }
        };
        deepest = deepest.max(nested);
    }
    deepest
}
