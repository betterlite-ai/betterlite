use super::{JsonObject, ParsedJson, RequestBodyError, parse_json_bytes, parse_json_text};
use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use std::str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedJson {
    Parsed { body: JsonObject, repaired: bool },
    // LiteLLM's lenient fallback decodes UTF-8 first, so unusable bytes leave auth with an empty body
    Unreadable,
}

pub fn decode_json_body(
    body: &[u8],
    limits: &ApiLimits,
    max_repair_bytes: Option<u64>,
) -> Result<DecodedJson, RequestBodyError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: body.len(),
    }))?;
    if body.is_empty() {
        return Ok(DecodedJson::Parsed {
            body: JsonObject::new(),
            repaired: false,
        });
    }
    match parse_json_bytes(body) {
        ParsedJson::Object(object) => Ok(DecodedJson::Parsed {
            body: object,
            repaired: false,
        }),
        ParsedJson::NotObject => Err(RequestBodyError::NotAnObject),
        ParsedJson::Invalid => repair_json_body(body, max_repair_bytes),
    }
}

// the cap only guards the lenient fallback, as in the pinned proxy: a plain parse still runs first
fn repair_json_body(
    body: &[u8],
    max_repair_bytes: Option<u64>,
) -> Result<DecodedJson, RequestBodyError> {
    if max_repair_bytes
        .is_some_and(|cap| usize::try_from(cap).is_ok_and(|limit| body.len() > limit))
    {
        return Err(RequestBodyError::InvalidJson);
    }
    let Ok(text) = str::from_utf8(body) else {
        return Ok(DecodedJson::Unreadable);
    };
    match parse_json_text(text) {
        ParsedJson::Object(object) => Ok(DecodedJson::Parsed {
            body: object,
            repaired: true,
        }),
        ParsedJson::NotObject => Err(RequestBodyError::NotAnObject),
        ParsedJson::Invalid => Err(RequestBodyError::InvalidJson),
    }
}
