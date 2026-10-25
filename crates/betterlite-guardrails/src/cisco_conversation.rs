use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;

pub mod json;
pub mod request;
pub mod response;
pub mod rewrite;
mod tests;

pub type Fields = IndexMap<String, Box<RawValue>>;

pub const TEXT_PART_TYPES: [&str; 5] = [
    "text",
    "input_text",
    "output_text",
    "summary_text",
    "reasoning_text",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    System,
    Assistant,
    #[serde(untagged)]
    Caller(Box<RawValue>),
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectMessage {
    pub role: Role,
    pub content: String,
}

#[must_use]
pub fn normalize_content(content: &Json) -> String {
    match content {
        Json::Null => String::new(),
        Json::List(items) => {
            let mut parts = Vec::new();
            for part in items {
                if let Json::Object(fields) = Json::parse(part.get()) {
                    push_part_text(&fields, &mut parts);
                }
            }
            parts.join(" ")
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => content.python_str(),
    }
}

fn push_part_text(part: &Fields, parts: &mut Vec<String>) {
    let text = Json::field(part, "text");
    if is_text_part(part) && text.truthy() {
        parts.push(text.python_str());
        return;
    }
    let nested = Json::field(part, "content");
    if !matches!(nested, Json::Null) {
        let nested_text = normalize_content(&nested);
        if !nested_text.is_empty() {
            parts.push(nested_text);
        }
    }
    for key in ["arguments", "output"] {
        let value = Json::field(part, key);
        // LiteLLM appends the normalized value even when it comes back empty
        if value.truthy() {
            parts.push(normalize_content(&value));
        }
    }
}

fn is_text_part(part: &Fields) -> bool {
    match Json::field(part, "type") {
        Json::Text(kind) => TEXT_PART_TYPES.contains(&kind.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }
}

fn caller_role(fields: &Fields) -> Option<Role> {
    match fields.get("role") {
        Some(role) if Json::parse(role.get()).truthy() => Some(Role::Caller(role.clone())),
        Some(_) | None => None,
    }
}
