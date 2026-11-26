pub mod agent;
pub mod cache;
pub mod clock;
pub mod errors;
pub mod http;
pub mod resolver;
pub mod settings;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;
pub const MAX_DOCUMENT_DEPTH: usize = 64;

// OIDC discovery, JWKS and UserInfo documents are provider JSON betterlite navigates, never a modeled wire type
pub type TypedValue = Json;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the json document is {bytes} bytes, over the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the json document nests deeper than {limit} levels")]
    TooDeep { limit: usize },
    #[error("the json document is not valid json")]
    Syntax(#[source] serde_json::Error),
    #[error("the {document} document is not a json object")]
    NotObject { document: &'static str },
}

// The one outside-byte boundary for an identity provider document: bounded before parsing and after.
pub fn decode_discovery_document(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DecodeError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    let value: TypedValue = serde_json::from_slice(bytes).map_err(DecodeError::Syntax)?;
    if exceeds_depth(&value, MAX_DOCUMENT_DEPTH) {
        return Err(DecodeError::TooDeep {
            limit: MAX_DOCUMENT_DEPTH,
        });
    }
    Ok(value)
}

// _userinfo_document: the UserInfo response decodes to its declared JSON object form or names what it is not.
pub fn userinfo_document(bytes: &[u8]) -> Result<IndexMap<String, Json>, DecodeError> {
    match decode_discovery_document(bytes)? {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(DecodeError::NotObject {
                document: "UserInfo",
            })
        }
    }
}

fn exceeds_depth(root: &TypedValue, limit: usize) -> bool {
    let mut pending = vec![(root, 1_usize)];
    while let Some((value, level)) = pending.pop() {
        match value {
            TypedValue::Object(fields) => {
                if level > limit {
                    return true;
                }
                pending.extend(
                    fields
                        .values()
                        .map(|child| (child, level.saturating_add(1))),
                );
            }
            TypedValue::List(items) => {
                if level > limit {
                    return true;
                }
                pending.extend(items.iter().map(|child| (child, level.saturating_add(1))));
            }
            TypedValue::Null
            | TypedValue::Bool(_)
            | TypedValue::Number(_)
            | TypedValue::Text(_) => {}
        }
    }
    false
}
