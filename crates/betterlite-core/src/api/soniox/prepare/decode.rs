use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde_json::from_slice;

use super::error::ReplyError;
use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};

pub const BYTE_ORDER_MARK: &[u8] = &[0xEF, 0xBB, 0xBF];
pub const ID_FIELD: &str = "id";

#[derive(Debug, Clone)]
pub struct SonioxReply {
    pub status: i64,
    pub headers: Headers,
    pub payload: Option<Object>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedResource {
    pub id: String,
}

impl SonioxReply {
    pub const fn object(&self) -> Result<&Object, ReplyError> {
        let Some(payload) = self.payload.as_ref() else {
            return Err(ReplyError::NotObject);
        };
        Ok(payload)
    }
}

pub fn decode_soniox_reply(
    status: i64,
    headers: Headers,
    body: &[u8],
    limits: &ApiLimits,
) -> Result<SonioxReply, ReplyError> {
    bounded(
        limits,
        ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: body.len(),
        }),
        |observed, limit| ReplyError::TooLarge { observed, limit },
    )?;
    let text = replaced_text(body);
    let payload = object_payload(body);
    Ok(SonioxReply {
        status,
        headers,
        payload,
        text,
    })
}

pub fn identified_resource(reply: &SonioxReply) -> Result<IdentifiedResource, ReplyError> {
    let object = reply.object()?;
    let Some(value) = object.get(ID_FIELD) else {
        return Err(ReplyError::Missing { field: ID_FIELD });
    };
    let Some(id) = value.as_str() else {
        return Err(ReplyError::Shape {
            field: ID_FIELD,
            expected: "a string",
        });
    };
    Ok(IdentifiedResource { id: id.to_owned() })
}

pub(super) fn bounded(
    limits: &ApiLimits,
    input: ApiBoundariesInput,
    rejection: fn(usize, usize) -> ReplyError,
) -> Result<(), ReplyError> {
    match limits.check(input) {
        Ok(_) => Ok(()),
        Err(
            ApiLimitError::JsonTooLarge { observed, limit }
            | ApiLimitError::EventTooLarge { observed, limit }
            | ApiLimitError::TooManyItems { observed, limit },
        ) => Err(rejection(observed, limit)),
    }
}

// httpx decodes the body with replacement characters, and json.loads skips a leading byte order mark
fn object_payload(body: &[u8]) -> Option<Object> {
    let payload = body.strip_prefix(BYTE_ORDER_MARK).unwrap_or(body);
    let Ok(decoded) = from_slice::<JsonValue>(payload) else {
        return None;
    };
    match decoded {
        JsonValue::Object(fields) => Some(fields),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn replaced_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
