use serde::Serialize;

use crate::arguments::json::{Json, Object};
use crate::bedrock_tool_results::invoke::CachePoints;

use super::conversation::Conversion;
use super::error::ProcessorError;

pub(crate) const ASSISTANT: &str = "assistant";
pub(crate) const CACHE_CONTROL: &str = "cache_control";
pub(crate) const CACHE_POINT: &str = "cachePoint";
pub(crate) const CONTENT: &str = "content";
pub(crate) const GUARDED_TEXT: &str = "guarded_text";
pub(crate) const IMAGE_URL: &str = "image_url";
pub(crate) const ROLE: &str = "role";
pub(crate) const TEXT: &str = "text";
pub(crate) const TOOL: &str = "tool";
pub(crate) const TOOL_CALLS: &str = "tool_calls";
pub(crate) const TYPE: &str = "type";
pub(crate) const USER: &str = "user";

#[derive(Clone, Copy)]
pub(crate) enum Role<'message> {
    Text(&'message str),
    Null,
}

pub(crate) const fn object_of(value: &Json) -> Option<&Object> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn role_at(message: &Json, index: usize) -> Result<Role<'_>, ProcessorError> {
    let fields = object_of(message).ok_or(ProcessorError::MessageNotObject { message: index })?;
    match fields.get(ROLE) {
        None => Err(ProcessorError::MessageWithoutRole { message: index }),
        Some(Json::Text(role)) => Ok(Role::Text(role.as_str())),
        Some(Json::Null) => Ok(Role::Null),
        Some(_) => Err(ProcessorError::RoleNotText { message: index }),
    }
}

pub(crate) fn block_kind(fields: &Object, block: usize) -> Result<&str, ProcessorError> {
    match fields.get(TYPE) {
        None => Err(ProcessorError::BlockWithoutType { block }),
        Some(Json::Text(kind)) => Ok(kind.as_str()),
        Some(_) => Err(ProcessorError::BlockTypeNotText { block }),
    }
}

pub(crate) fn block_text(fields: &Object, block: usize) -> Result<&str, ProcessorError> {
    match fields.get(TEXT) {
        None => Err(ProcessorError::TextWithoutText { block }),
        Some(Json::Text(text)) => Ok(text.as_str()),
        Some(_) => Err(ProcessorError::TextNotText { block }),
    }
}

// Python's str.strip() also treats U+001C-U+001F as whitespace, which Rust's is_whitespace() does not.
pub(crate) fn is_blank(text: &str) -> bool {
    text.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    })
    .is_empty()
}

pub(crate) fn push_cache_point<C>(
    blocks: &mut Vec<Json>,
    control: &Json,
    cache_points: &C,
    model: Option<&str>,
) where
    C: CachePoints,
{
    let Some(point) = cache_points.cache_point_block(control, model) else {
        return;
    };
    blocks.push(Json::Object(Object::from_iter([(
        CACHE_POINT.to_owned(),
        Json::Object(point),
    )])));
}

pub(crate) fn append_cache_point<C, R>(
    blocks: &mut Vec<Json>,
    fields: &Object,
    conversion: &Conversion<'_, C, R>,
) where
    C: CachePoints,
{
    let Some(control) = fields.get(CACHE_CONTROL) else {
        return;
    };
    if matches!(control, Json::Null) {
        return;
    }
    push_cache_point(blocks, control, conversion.cache_points, conversion.model);
}

pub(crate) fn as_json<Payload>(payload: &Payload) -> Result<Json, ProcessorError>
where
    Payload: Serialize,
{
    let value =
        serde_json::to_value(payload).map_err(|source| ProcessorError::Serialization { source })?;
    serde_json::from_value(value).map_err(|source| ProcessorError::Serialization { source })
}

#[must_use]
pub(crate) fn text_block(text: &str) -> Json {
    let mut block = Object::new();
    block.insert(TEXT.to_owned(), Json::Text(text.to_owned()));
    Json::Object(block)
}

#[must_use]
pub(crate) fn message(role: &str, content: Vec<Json>) -> Json {
    Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(role.to_owned())),
        (CONTENT.to_owned(), Json::List(content)),
    ]))
}

#[must_use]
pub(crate) const fn continue_present(continue_message: Option<&Json>) -> bool {
    match continue_message {
        Some(Json::Null) | None => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            true
        }
    }
}
