use std::str;
use std::str::Utf8Error;

use betterlite_models::controls::json_value::JsonValue;
use thiserror::Error;

use super::TypedValue;
use crate::jwt_claims::decode::{
    DecodeError as ClaimsDecodeError, decode_segment, decode_unverified_claims,
};

// The bounded outside-byte decoder this task owns: a compact token, a JWK or a signed claim document.
pub const MAX_SIGNED_JWT_BYTES: usize = 0x0010_0000;
pub const MAX_SIGNED_JWT_DEPTH: usize = 64;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the signed JWT input is {bytes} bytes, over the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the signed JWT input nests deeper than {limit} JSON levels")]
    TooDeep { limit: usize },
    #[error("the signed JWT input is not valid UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("the signed JWT input has fewer than three segments")]
    Segments,
    #[error("the JWT header is not a usable JSON object")]
    Header,
    #[error(transparent)]
    Claims(#[from] ClaimsDecodeError),
    #[error("the signed JWT input is neither a JSON object nor a compact token")]
    NotDocument,
    #[error("the signed JWT document could not be rebuilt as a typed value")]
    Document(#[source] serde_json::Error),
}

// JWTHandler.get_unverified_header reads alg and kid off pyjwt's _load, which also validates kid, crit and b64
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JwtHeader {
    pub algorithm: Option<String>,
    pub kid: Option<String>,
}

impl JwtHeader {
    #[must_use]
    pub fn parse(token: &str) -> Result<Self, DecodeError> {
        if token.len() > MAX_SIGNED_JWT_BYTES {
            return Err(DecodeError::TooLarge {
                bytes: token.len(),
                limit: MAX_SIGNED_JWT_BYTES,
            });
        }
        let Some((signing_input, signature)) = token.rsplit_once('.') else {
            return Err(DecodeError::Segments);
        };
        let Some((header_segment, payload_segment)) = signing_input.split_once('.') else {
            return Err(DecodeError::Segments);
        };
        let header_bytes = decode_segment(header_segment.as_bytes())?;
        let Ok(JsonValue::Object(fields)) = serde_json::from_slice::<JsonValue>(&header_bytes)
        else {
            return Err(DecodeError::Header);
        };
        // pyjwt rejects a detached payload that still carries base64url text, then decodes the rest
        let detached = matches!(fields.get("b64"), Some(JsonValue::Bool(false)));
        if detached && !payload_segment.is_empty() {
            return Err(DecodeError::Header);
        }
        if !detached {
            decode_segment(payload_segment.as_bytes())?;
        }
        decode_segment(signature.as_bytes())?;
        // pyjwt reads a present null as a value: a null kid is not a string, so the header is invalid
        if let Some(kid) = fields.get("kid")
            && !matches!(kid, JsonValue::Text(_))
        {
            return Err(DecodeError::Header);
        }
        validate_crit(fields.get("crit"), fields.contains_key("b64"))?;
        Ok(Self {
            algorithm: text_of(fields.get("alg")),
            kid: text_of(fields.get("kid")),
        })
    }
}

// pyjwt's _validate_crit accepts only a non-empty list naming "b64", and that parameter must be present
fn validate_crit(crit: Option<&JsonValue>, b64_present: bool) -> Result<(), DecodeError> {
    let Some(crit) = crit else {
        return Ok(());
    };
    let JsonValue::Array(extensions) = crit else {
        return Err(DecodeError::Header);
    };
    if extensions.is_empty() {
        return Err(DecodeError::Header);
    }
    for extension in extensions {
        let JsonValue::Text(name) = extension else {
            return Err(DecodeError::Header);
        };
        if name != "b64" || !b64_present {
            return Err(DecodeError::Header);
        }
    }
    Ok(())
}

fn text_of(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::Text(text)) => Some(text.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

// Production calls this before any typed policy: a JSON object (JWK or claim document) or a compact token's payload.
pub fn decode_signed_jwt(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_SIGNED_JWT_BYTES {
        return Err(DecodeError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_SIGNED_JWT_BYTES,
        });
    }
    if let Ok(tree) = serde_json::from_slice::<JsonValue>(bytes) {
        if nesting_level(&tree) > MAX_SIGNED_JWT_DEPTH {
            return Err(DecodeError::TooDeep {
                limit: MAX_SIGNED_JWT_DEPTH,
            });
        }
        if !matches!(tree, JsonValue::Object(_)) {
            return Err(DecodeError::NotDocument);
        }
        let json = serde_json::to_value(&tree).map_err(DecodeError::Document)?;
        return Ok(TypedValue { json });
    }
    let token = str::from_utf8(bytes).map_err(DecodeError::Utf8)?;
    JwtHeader::parse(token)?;
    let Some((signing_input, _signature)) = token.rsplit_once('.') else {
        return Err(DecodeError::Segments);
    };
    let Some((_header, payload)) = signing_input.split_once('.') else {
        return Err(DecodeError::Segments);
    };
    decode_unverified_claims(payload.as_bytes()).map_err(DecodeError::Claims)
}

// serde_json refuses documents past its own recursion limit, so the walk below never sees more than 128 levels
fn nesting_level(value: &JsonValue) -> usize {
    match value {
        JsonValue::Array(items) => items
            .iter()
            .map(nesting_level)
            .max()
            .map_or(1_usize, |deepest| deepest.saturating_add(1)),
        JsonValue::Object(fields) => fields
            .values()
            .map(nesting_level)
            .max()
            .map_or(1_usize, |deepest| deepest.saturating_add(1)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => 0,
    }
}
