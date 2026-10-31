pub mod any_messages;
pub mod image;
pub mod projection;
pub mod prompt;

use crate::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub fields: Object,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    // LiteLLM's message dicts omit "content"/"name" entirely rather than writing null when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    // LiteLLM forwards provider-specific message keys, including search_results, without interpreting them.
    #[serde(flatten)]
    pub provider_specific_fields: Object,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContentOnlyMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContentError {
    #[error(
        "`prompt` must be a non-empty string or a non-empty list of strings. Got: {prompt_type}."
    )]
    InvalidPrompt { prompt_type: String },
}

#[must_use]
pub fn convert_message_content_lists(messages: &mut [Message]) -> Vec<Message> {
    let mut converted = Vec::with_capacity(messages.len());
    for original in messages.iter() {
        let mut updated = original.clone();
        let text = convert_content_list_to_str(&updated);
        if !text.is_empty() {
            updated.content = Some(Content::Text(text));
        }
        converted.push(updated);
    }
    converted
}

#[must_use]
pub fn strip_name_from_message(message: &Message, allowed_name_roles: &[String]) -> Message {
    let mut stripped = message.clone();
    if !allowed_name_roles.iter().any(|role| role == &stripped.role) {
        stripped.name = None;
    }
    stripped
}

#[must_use]
pub fn strip_name_from_messages(
    messages: &[Message],
    allowed_name_roles: &[String],
) -> Vec<Message> {
    let mut stripped = Vec::with_capacity(messages.len());
    for message in messages {
        stripped.push(strip_name_from_message(message, allowed_name_roles));
    }
    stripped
}

#[must_use]
pub fn strip_none_values_from_message(message: &Message) -> Message {
    let mut stripped = message.clone();
    stripped
        .provider_specific_fields
        .retain(|_, value| !matches!(value, Json::Null));
    stripped
}

#[must_use]
pub fn extract_search_results_text(search_results: Option<&Json>) -> String {
    let Some(Json::List(results)) = search_results else {
        return String::new();
    };
    let mut text = String::new();
    for result in results {
        let Json::Object(fields) = result else {
            continue;
        };
        for key in ["source", "title"] {
            if let Some(value) = fields.get(key).and_then(Json::as_str) {
                text.push_str(value);
            }
        }
        if let Some(Json::List(content)) = fields.get("content") {
            push_content_blocks_text(&mut text, content);
        }
        if let Some(citations) = fields
            .get("citations")
            .filter(|value| !matches!(value, Json::Null))
        {
            text.push_str(&serde_json::to_string(citations).unwrap_or_default());
        }
    }
    text
}

fn push_content_blocks_text(text: &mut String, content: &[Json]) {
    for block in content {
        let Json::Object(fields) = block else {
            continue;
        };
        if let Some(value) = fields.get("text").and_then(Json::as_str) {
            text.push_str(value);
        }
    }
}

#[must_use]
pub fn convert_content_list_to_str(message: &Message) -> String {
    let mut text = match message.content.as_ref() {
        Some(Content::Text(value)) => value.clone(),
        Some(Content::Parts(parts)) => parts
            .iter()
            .filter_map(|part| part.fields.get("text").and_then(Json::as_str))
            .collect(),
        None => String::new(),
    };
    text.push_str(&extract_search_results_text(
        message.provider_specific_fields.get("search_results"),
    ));
    text
}

#[must_use]
pub fn get_str_from_messages(messages: &[Message]) -> String {
    let mut text = String::new();
    for message in messages {
        text.push_str(&convert_content_list_to_str(message));
    }
    text
}

#[must_use]
pub fn is_non_content_values_set(message: &Message) -> bool {
    // LiteLLM's ignore_keys excludes "name" along with "content" and "role"; a null value doesn't count as set.
    message
        .provider_specific_fields
        .values()
        .any(|value| !matches!(value, Json::Null))
}

// `kind` lives outside `fields`, so a part walked back into raw JSON needs "type" reinserted.
pub(crate) fn as_fields(part: &ContentPart) -> Object {
    let mut fields = part.fields.clone();
    fields.insert("type".to_owned(), Json::Text(part.kind.clone()));
    fields
}

mod tests;
