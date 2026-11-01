use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::blocking::StructuredMirror;
use crate::cisco_mcp::normalize::{is_pair_list, pairs_to_fields};

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ContentItemUpdate {
    TextReplaced {
        #[serde(flatten)]
        fields: Fields,
        text: String,
    },
    Unchanged(Box<RawValue>),
}

#[derive(Debug, Clone)]
pub struct TextUpdate {
    pub content: Option<Vec<ContentItemUpdate>>,
    pub structured_content: Option<StructuredMirror>,
    pub replaced: bool,
}

#[must_use]
pub fn set_tool_response_text(response: &Json, text: &str) -> TextUpdate {
    let target = unwrap_wrapper(response);
    let content = coerce_to_content_list(&target).map(|items| replace_text_items(&items, text));
    let content_replaced = content.as_ref().is_some_and(|items| {
        items
            .iter()
            .any(|item| matches!(item, ContentItemUpdate::TextReplaced { .. }))
    });
    let (structured_content, structured_replaced) = update_structured_mirror(&target, text);
    TextUpdate {
        content,
        structured_content,
        replaced: content_replaced || structured_replaced,
    }
}

fn unwrap_wrapper(response: &Json) -> Json {
    let mut current = response.clone();
    loop {
        let Json::Object(fields) = &current else {
            return current;
        };
        match fields.get("mcp_tool_call_response") {
            Some(raw) if !matches!(Json::parse(raw.get()), Json::Null) => {
                current = Json::parse(raw.get());
            }
            _ => return current,
        }
    }
}

#[must_use]
pub fn coerce_to_content_list(response: &Json) -> Option<Vec<Box<RawValue>>> {
    match response {
        Json::Object(fields) => match Json::field(fields, "content") {
            Json::List(items) => Some(items),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
        },
        Json::List(items) => {
            if is_pair_list(items) {
                let fields = pairs_to_fields(items);
                match Json::field(&fields, "content") {
                    Json::List(inner) => Some(inner),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::Object(_) => None,
                }
            } else {
                Some(items.clone())
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
    }
}

#[must_use]
pub fn replace_text_items(items: &[Box<RawValue>], text: &str) -> Vec<ContentItemUpdate> {
    let mut updated = Vec::with_capacity(items.len());
    for item in items {
        updated.push(replace_text_item(item, text));
    }
    updated
}

fn replace_text_item(item: &RawValue, text: &str) -> ContentItemUpdate {
    if let Json::Object(mut fields) = Json::parse(item.get())
        && matches!(Json::field(&fields, "type"), Json::Text(kind) if kind == "text")
    {
        fields.shift_remove("text");
        return ContentItemUpdate::TextReplaced {
            fields,
            text: text.to_owned(),
        };
    }
    ContentItemUpdate::Unchanged(item.to_owned())
}

fn update_structured_mirror(target: &Json, text: &str) -> (Option<StructuredMirror>, bool) {
    let has_structured_key = match target {
        Json::List(items) if is_pair_list(items) => {
            let fields = pairs_to_fields(items);
            fields.contains_key("structuredContent") || fields.contains_key("structured_content")
        }
        Json::Object(fields) => {
            let result_target = match Json::field(fields, "result") {
                Json::Object(result_fields) => result_fields,
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    fields.clone()
                }
            };
            result_target.contains_key("structured_content")
                || result_target.contains_key("structuredContent")
        }
        Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => false,
    };
    if has_structured_key {
        (
            Some(StructuredMirror {
                result: text.to_owned(),
            }),
            true,
        )
    } else {
        (None, false)
    }
}

mod tests;
