use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::limits::MAX_MESSAGE_PARTS;
use serde::Serialize;

use crate::gemini_history::error::HistoryError;
use crate::gemini_media::process::{MediaRequest, process_media};

macro_rules! object {
    ($($key:expr => $value:expr),* $(,)?) => {
        Object::from_iter([$(($key.to_owned(), $value)),*])
    };
}
pub(crate) use object;

pub const FORMAT: &str = "format";
pub const MIME_TYPE: &str = "mime_type";
pub const CONTENT_TYPE: &str = "content_type";
pub const DETAIL: &str = "detail";
const SIGNATURE_KEY: &str = "thoughtSignature";
pub const MAX_THINKING_BYTES: usize = 0x0010_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Model,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Content {
    pub role: Role,
    pub parts: Vec<Object>,
}

#[must_use]
pub fn decode_thinking_part(thinking: &str, signature: &str) -> Object {
    if thinking.len() <= MAX_THINKING_BYTES
        && let Ok(Json::Object(mut object)) = serde_json::from_str::<Json>(thinking)
        && !object.contains_key(SIGNATURE_KEY)
    {
        object.insert(SIGNATURE_KEY.to_owned(), Json::Text(signature.to_owned()));
        return object;
    }
    object!("text" => Json::Text(thinking.to_owned()), SIGNATURE_KEY => Json::Text(signature.to_owned()))
}

pub fn object_of<T>(value: T) -> Result<Object, HistoryError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(HistoryError::Encode)?;
    match serde_json::from_value::<Json>(encoded).map_err(HistoryError::Encode)? {
        Json::Object(object) => Ok(object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Err(
            HistoryError::Invalid("a content part did not encode as an object".to_owned()),
        ),
    }
}

pub fn push_media(parts: &mut Vec<Object>, request: &MediaRequest<'_>) -> Result<(), HistoryError> {
    parts.push(object_of(process_media(request)?)?);
    Ok(())
}

pub const fn bounded<Item>(items: &[Item]) -> Result<&[Item], HistoryError> {
    if items.len() > MAX_MESSAGE_PARTS {
        return Err(HistoryError::TooManyParts(MAX_MESSAGE_PARTS));
    }
    Ok(items)
}
