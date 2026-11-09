use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::galileo_payloads::messages::convert_content_list_to_str;

use super::records::{DdMessage, InputMessage, Message};
use super::tool_calls::to_dd_tool_calls;
use super::tool_results::{to_dd_tool_results, tool_call_names_by_id};
use super::values::{Object, as_object, text_field};

// LiteLLM's _to_dd_messages: a whole conversation, with each tool result resolved against its call.
#[must_use]
pub fn to_dd_messages(messages: Option<&Json>) -> Vec<DdMessage> {
    let Some(conversation) = messages else {
        return Vec::new();
    };
    if matches!(conversation, Json::Null) {
        return Vec::new();
    }
    let Json::List(items) = conversation else {
        return converted_messages(conversation)
            .into_iter()
            .map(as_dd_message)
            .collect();
    };
    let tool_call_names = tool_call_names_by_id(items);
    items
        .iter()
        .map(|message| to_dd_message(message, &tool_call_names))
        .collect()
}

// LiteLLM's _to_dd_message: a dict is projected, anything else goes through its conversion fallbacks.
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
    converted_messages(value).into_iter().next().map_or_else(
        // LiteLLM's _EMPTY_MESSAGE is an empty message dict, which projects to itself
        || DdMessage::Message(project_message(&IndexMap::new(), tool_call_names)),
        as_dd_message,
    )
}

fn project_message(fields: &Object, tool_call_names: &IndexMap<String, String>) -> Message {
    let text = convert_content_list_to_str(fields);
    let content = if text.is_empty() {
        match fields.get("content") {
            Some(Json::List(blocks)) if !blocks.is_empty() => Json::List(blocks.clone()),
            Some(
                Json::List(_)
                | Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::Object(_),
            )
            | None => Json::Text(String::new()),
        }
    } else {
        Json::Text(text)
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

enum Converted {
    Input(String),
    Fields(Object),
}

fn as_dd_message(converted: Converted) -> DdMessage {
    match converted {
        Converted::Input(input) => DdMessage::Input(InputMessage { input }),
        Converted::Fields(fields) => DdMessage::Raw(fields),
    }
}

// litellm's handle_any_messages_to_chat_completion_str_messages_conversion.
fn converted_messages(value: &Json) -> Vec<Converted> {
    match value {
        Json::List(items) => converted_list(items),
        Json::Text(text) => vec![Converted::Input(text.clone())],
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            vec![Converted::Input(
                serde_json::to_string(value).unwrap_or_default(),
            )]
        }
    }
}

fn converted_list(items: &[Json]) -> Vec<Converted> {
    let converted: Option<Vec<Object>> = items.iter().map(converted_fields).collect();
    converted.map_or_else(
        || {
            items
                .iter()
                .map(|item| Converted::Input(serde_json::to_string(item).unwrap_or_default()))
                .collect()
        },
        |messages| messages.into_iter().map(Converted::Fields).collect(),
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
