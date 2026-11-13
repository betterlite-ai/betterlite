use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent};
use indexmap::IndexMap;

#[must_use]
pub fn content_blocks(message: &ChatMessage) -> &[ContentPart] {
    match &message.content {
        Some(MessageContent::Parts(parts)) => parts,
        Some(MessageContent::Text(_)) | None => &[],
    }
}

pub fn field_text<'a>(fields: &'a IndexMap<String, Json>, key: &str) -> Option<&'a str> {
    match fields.get(key) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) | None => None,
    }
}

// LiteLLM only tests thinking_blocks for truthiness, so an empty list or a zero counts as absent
pub(crate) fn json_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|number| number != 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
