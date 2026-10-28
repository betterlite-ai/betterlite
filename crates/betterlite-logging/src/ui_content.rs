use std::slice;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::message_content::{ChatRole, content_text};
use crate::python_json::dumps;

mod tests;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiToolCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<UiToolCall>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiField {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UiContent {
    Messages { messages: Vec<UiMessage> },
    Fields { fields: Vec<UiField> },
    Text { text: String },
}

// A message as LiteLLM's _RawMessage validates it, with a LangChain kwargs wrapper not yet unwrapped
struct RawMessage<'json> {
    role: Option<&'json str>,
    kind: Option<&'json str>,
    content: Option<&'json Json>,
    name: Option<&'json str>,
    tool_calls: Vec<UiToolCall>,
    kwargs: Option<Box<Self>>,
}

#[must_use]
pub fn to_ui_content(raw: &str) -> UiContent {
    let Ok(parsed) = serde_json::from_str::<Json>(raw) else {
        return UiContent::Text {
            text: raw.to_owned(),
        };
    };
    let converted = match &parsed {
        Json::List(items) => ui_messages(items),
        Json::Object(_) => ui_messages(slice::from_ref(&parsed)),
        Json::Text(text) => return UiContent::Text { text: text.clone() },
        Json::Null | Json::Bool(_) | Json::Number(_) => {
            return UiContent::Text {
                text: raw.to_owned(),
            };
        }
    };
    match (converted, parsed) {
        (Some(messages), _) => UiContent::Messages { messages },
        (None, Json::Object(fields)) => UiContent::Fields {
            fields: fields
                .into_iter()
                .map(|(key, value)| UiField {
                    value: if let Json::Text(text) = value {
                        text
                    } else {
                        dumps(&value)
                    },
                    key,
                })
                .collect(),
        },
        (None, Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            UiContent::Text {
                text: raw.to_owned(),
            }
        }
    }
}

fn ui_messages(items: &[Json]) -> Option<Vec<UiMessage>> {
    let validated: Vec<RawMessage<'_>> = items
        .iter()
        .map(raw_message)
        .collect::<Result<_, _>>()
        .ok()?;
    let unwrapped: Vec<RawMessage<'_>> = validated
        .into_iter()
        .map(|message| match message.kwargs {
            Some(inner) => *inner,
            None => message,
        })
        .collect();
    if unwrapped.is_empty() || !unwrapped.iter().all(is_message) {
        return None;
    }
    Some(unwrapped.into_iter().map(ui_message).collect())
}

fn is_message(message: &RawMessage<'_>) -> bool {
    let has_role = message.role.is_some() || message.kind.and_then(ChatRole::from_name).is_some();
    has_role && (message.content.is_some() || !message.tool_calls.is_empty())
}

fn ui_message(message: RawMessage<'_>) -> UiMessage {
    // An empty role falls through to the type, and an unknown one means the assistant only when it calls tools
    let known = [message.role, message.kind]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .and_then(ChatRole::from_name);
    let fallback = if message.tool_calls.is_empty() {
        ChatRole::User
    } else {
        ChatRole::Assistant
    };
    UiMessage {
        role: known.unwrap_or(fallback),
        content: content_text(message.content.unwrap_or(&Json::Null)),
        name: message
            .name
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        tool_calls: message.tool_calls,
    }
}

// Pydantic rejected the value, so the input is not a message list
struct Invalid;

const fn as_object(value: &Json) -> Result<&IndexMap<String, Json>, Invalid> {
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(Invalid)
        }
    }
}

fn raw_message(value: &Json) -> Result<RawMessage<'_>, Invalid> {
    let fields = as_object(value)?;
    let tool_calls = match fields.get("tool_calls") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(calls)) => calls.iter().map(tool_call).collect::<Result<_, _>>()?,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(Invalid);
        }
    };
    let kwargs = match fields.get("kwargs") {
        None | Some(Json::Null) => None,
        Some(inner) => Some(Box::new(raw_message(inner)?)),
    };
    Ok(RawMessage {
        role: optional_text(fields, "role")?,
        kind: optional_text(fields, "type")?,
        content: fields.get("content"),
        name: optional_text(fields, "name")?,
        tool_calls,
        kwargs,
    })
}

// A LangChain call nests the real name and arguments under "function"; OpenAI's "args" is the fallback.
fn tool_name_and_arguments<'json>(
    fields: &'json IndexMap<String, Json>,
    call_name: &'json str,
) -> Result<(&'json str, Option<&'json Json>), Invalid> {
    let Some(function) = fields.get("function") else {
        return Ok(fallback_arguments(fields, call_name));
    };
    match function {
        Json::Null => Ok(fallback_arguments(fields, call_name)),
        Json::Object(inner) => {
            let function_name = required_text(inner, "name")?;
            let name = if function_name.is_empty() {
                call_name
            } else {
                function_name
            };
            Ok((name, inner.get("arguments")))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Err(Invalid),
    }
}

fn fallback_arguments<'json>(
    fields: &'json IndexMap<String, Json>,
    call_name: &'json str,
) -> (&'json str, Option<&'json Json>) {
    match fields.get("args") {
        None | Some(Json::Null) => (call_name, fields.get("arguments")),
        args => (call_name, args),
    }
}

fn argument_text(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => "{}".to_owned(),
        Some(Json::Text(text)) => text.clone(),
        Some(other @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))) => {
            dumps(other)
        }
    }
}

fn tool_call(value: &Json) -> Result<UiToolCall, Invalid> {
    let fields = as_object(value)?;
    let call_name = required_text(fields, "name")?;
    let (name, arguments) = tool_name_and_arguments(fields, call_name)?;
    Ok(UiToolCall {
        name: name.to_owned(),
        arguments: argument_text(arguments),
    })
}

const fn as_text(value: &Json) -> Result<&str, Invalid> {
    match value {
        Json::Text(text) => Ok(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(Invalid)
        }
    }
}

// Pydantic's str | None: a missing key and null are None, any other non-string fails validation
fn optional_text<'json>(
    fields: &'json IndexMap<String, Json>,
    key: &str,
) -> Result<Option<&'json str>, Invalid> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(other) => as_text(other).map(Some),
    }
}

// Pydantic's str with an empty default: only a missing key takes the default, so null fails validation
fn required_text<'json>(
    fields: &'json IndexMap<String, Json>,
    key: &str,
) -> Result<&'json str, Invalid> {
    match fields.get(key) {
        None => Ok(""),
        Some(value) => as_text(value),
    }
}
