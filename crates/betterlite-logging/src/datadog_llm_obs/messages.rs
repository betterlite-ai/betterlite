use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::galileo_payloads::messages::convert_content_list_to_str;

use super::records::{DdMessage, InputMessage, Message};
use super::tool_calls::to_dd_tool_calls;
use super::tool_results::{to_dd_tool_results, tool_call_names_by_id};
use super::values::{Object, as_object, as_text, text_field};

#[must_use]
pub fn to_dd_messages(messages: Option<&Json>) -> Vec<DdMessage> {
    let Some(conversation) = messages.filter(|value| !matches!(value, Json::Null)) else {
        return Vec::new();
    };
    let Json::List(items) = conversation else {
        return converted_messages(conversation);
    };
    let tool_call_names = tool_call_names_by_id(items);
    items
        .iter()
        .map(|message| to_dd_message(message, &tool_call_names))
        .collect()
}

#[must_use]
pub fn to_dd_message(message: &Json, tool_call_names: &IndexMap<String, String>) -> DdMessage {
    match message {
        Json::Object(fields) => DdMessage::Message(project_message(fields, tool_call_names)),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            converted_dd_message(other, tool_call_names)
        }
    }
}

fn converted_dd_message(value: &Json, tool_call_names: &IndexMap<String, String>) -> DdMessage {
    converted_messages(value)
        .into_iter()
        .next()
        .unwrap_or_else(|| DdMessage::Message(project_message(&IndexMap::new(), tool_call_names)))
}

fn project_message(fields: &Object, tool_call_names: &IndexMap<String, String>) -> Message {
    let text = convert_content_list_to_str(fields);
    let content = match fields.get("content") {
        Some(Json::List(blocks)) if text.is_empty() && !blocks.is_empty() => {
            Json::List(blocks.clone())
        }
        _ if !text.is_empty() => Json::Text(text),
        _ => Json::Text(String::new()),
    };
    Message {
        role: text_field(fields, "role", ""),
        content,
        reasoning_content: fields
            .get("reasoning_content")
            .filter(|value| !matches!(value, Json::Null))
            .cloned(),
        tool_calls: to_dd_tool_calls(fields),
        tool_results: to_dd_tool_results(fields, tool_call_names),
    }
}

fn converted_messages(value: &Json) -> Vec<DdMessage> {
    if let Json::List(items) = value {
        return converted_list(items);
    }
    let input = as_text(value).map_or_else(
        || serde_json::to_string(value).unwrap_or_default(),
        str::to_owned,
    );
    vec![DdMessage::Input(InputMessage { input })]
}

fn converted_list(items: &[Json]) -> Vec<DdMessage> {
    let converted: Option<Vec<Object>> = items.iter().map(converted_fields).collect();
    converted.map_or_else(
        || {
            items
                .iter()
                .map(|item| {
                    let input = serde_json::to_string(item).unwrap_or_default();
                    DdMessage::Input(InputMessage { input })
                })
                .collect()
        },
        |messages| messages.into_iter().map(DdMessage::Raw).collect(),
    )
}

// A content list LiteLLM cannot read raises inside its conversion, which falls the whole list back to JSON.
fn converted_fields(item: &Json) -> Option<Object> {
    let mut message = as_object(item)?.clone();
    if let Some(Json::List(blocks)) = message.get("content")
        && blocks.iter().any(|block| !matches!(block, Json::Object(_)))
    {
        return None;
    }
    let text = convert_content_list_to_str(&message);
    if !text.is_empty() {
        message.insert("content".to_owned(), Json::Text(text));
    }
    Some(message)
}
