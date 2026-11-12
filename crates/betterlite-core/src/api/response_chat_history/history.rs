use std::collections::BTreeSet;

use betterlite_logging::analytics::python;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::error::HistoryError;
use super::reasoning::ReasoningIds;
use crate::api::response_chat_content::content::text_by_role;
use crate::api::responses_input_content::is_truthy;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HistoryMessage {
    pub role: String,
    // Python's msg.get("content", "") tells an absent key (the empty string) from an explicit null
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub content: Option<Option<Value>>,
    #[serde(default)]
    pub tool_calls: Option<Vec<Value>>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub reasoning_items: Option<Vec<Map<String, Value>>>,
    #[serde(default)]
    pub thinking_blocks: Option<Vec<Map<String, Value>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConvertedHistory {
    pub input: Vec<Value>,
    pub instructions: Option<String>,
}

struct HistoryConversion {
    custom_call_ids: BTreeSet<String>,
    reasoning_ids: ReasoningIds,
    input: Vec<Value>,
    instructions: Option<String>,
}

pub fn convert_history(messages: &[HistoryMessage]) -> Result<ConvertedHistory, HistoryError> {
    let mut conversion = HistoryConversion {
        custom_call_ids: custom_call_ids(messages),
        reasoning_ids: ReasoningIds::default(),
        input: Vec::new(),
        instructions: None,
    };
    let mut leading = true;
    for message in messages {
        conversion.append(message, leading)?;
        if message.role != "system" {
            leading = false;
        }
    }
    Ok(ConvertedHistory {
        input: conversion.input,
        instructions: conversion.instructions,
    })
}

// A result can arrive before the assistant turn that defines a custom call, so the whole history is scanned first
fn custom_call_ids(messages: &[HistoryMessage]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for message in messages {
        if message.role != "assistant" {
            continue;
        }
        let Some(tool_calls) = &message.tool_calls else {
            continue;
        };
        for tool_call in tool_calls {
            let Some(object) = tool_call.as_object() else {
                continue;
            };
            let function_is_truthy = object
                .get("function")
                .is_some_and(|function| is_truthy(Some(function)));
            let custom_is_dict = matches!(object.get("custom"), Some(Value::Object(_)));
            if function_is_truthy || !custom_is_dict {
                continue;
            }
            if let Some(Value::String(id)) = object.get("id") {
                ids.insert(id.clone());
            }
        }
    }
    ids
}

impl HistoryConversion {
    fn append(&mut self, message: &HistoryMessage, leading: bool) -> Result<(), HistoryError> {
        let empty_content = Value::String(String::new());
        let content = match &message.content {
            None => Some(&empty_content),
            Some(None) => None,
            Some(Some(value)) => Some(value),
        };
        if message.role == "system" {
            return self.append_system(leading, content);
        }
        if message.role == "tool" {
            return self.append_tool(message, content);
        }
        let has_calls = message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
        if message.role == "assistant" && has_calls {
            return self.append_assistant_calls(message, content);
        }
        self.append_content_message(message, content)
    }

    fn append_system(
        &mut self,
        leading: bool,
        content: Option<&Value>,
    ) -> Result<(), HistoryError> {
        if leading && let Some(Value::String(text)) = content {
            let joined = match &self.instructions {
                Some(existing) if !existing.is_empty() => format!("{existing} {text}"),
                Some(_) | None => text.clone(),
            };
            self.instructions = Some(joined);
            return Ok(());
        }
        let converted = text_by_role(content.unwrap_or(&Value::Null), "system")?;
        self.push_message("system", converted);
        Ok(())
    }

    fn append_tool(
        &mut self,
        message: &HistoryMessage,
        content: Option<&Value>,
    ) -> Result<(), HistoryError> {
        let tool_output = match content {
            None => Value::Array(Vec::new()),
            Some(value) => text_by_role(value, "user")?,
        };
        let call_id = message.tool_call_id.as_deref();
        let custom = call_id.is_some_and(|id| self.custom_call_ids.contains(id));
        let output = match content {
            Some(Value::String(text)) if custom => Value::String(text.clone()),
            _ => tool_output,
        };
        let kind = if custom {
            "custom_tool_call_output"
        } else {
            "function_call_output"
        };
        let mut item = Map::new();
        item.insert("type".to_owned(), Value::String(kind.to_owned()));
        item.insert(
            "call_id".to_owned(),
            call_id.map_or(Value::Null, |id| Value::String(id.to_owned())),
        );
        item.insert("output".to_owned(), output);
        self.input.push(Value::Object(item));
        Ok(())
    }

    fn append_assistant_calls(
        &mut self,
        message: &HistoryMessage,
        content: Option<&Value>,
    ) -> Result<(), HistoryError> {
        self.append_reasoning(message)?;
        if let Some(value) = content
            && is_truthy(Some(value))
        {
            let converted = text_by_role(value, "assistant")?;
            self.push_message("assistant", converted);
        }
        for tool_call in message.tool_calls.iter().flatten() {
            self.append_tool_call(tool_call)?;
        }
        Ok(())
    }

    fn append_tool_call(&mut self, tool_call: &Value) -> Result<(), HistoryError> {
        let object = tool_call.as_object().ok_or(HistoryError::ToolCallShape)?;
        let call_id = object
            .get("id")
            .and_then(Value::as_str)
            .ok_or(HistoryError::ToolCallShape)?;
        if object
            .get("function")
            .is_some_and(|function| is_truthy(Some(function)))
        {
            let fields = match object.get("function") {
                Some(Value::Object(function)) => ["name", "arguments"]
                    .into_iter()
                    .filter_map(|field| function.get(field).map(|value| (field, value.clone())))
                    .collect(),
                _ => Vec::new(),
            };
            self.push_tool_call_item("function_call", call_id, fields);
            return Ok(());
        }
        if let Some(Value::Object(custom)) = object.get("custom") {
            let fields = ["name", "input"]
                .into_iter()
                .map(|field| {
                    let value = custom
                        .get(field)
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()));
                    (field, value)
                })
                .collect();
            self.push_tool_call_item("custom_tool_call", call_id, fields);
            return Ok(());
        }
        let unsupported =
            python::repr(tool_call).map_err(|source| HistoryError::ToolCallPrint { source })?;
        Err(HistoryError::UnsupportedToolCall {
            tool_call: unsupported,
        })
    }

    fn push_tool_call_item(
        &mut self,
        kind: &str,
        call_id: &str,
        fields: Vec<(&'static str, Value)>,
    ) {
        let mut item = Map::new();
        item.insert("type".to_owned(), Value::String(kind.to_owned()));
        item.insert("call_id".to_owned(), Value::String(call_id.to_owned()));
        for (field, value) in fields {
            item.insert(field.to_owned(), value);
        }
        self.input.push(Value::Object(item));
    }

    fn append_content_message(
        &mut self,
        message: &HistoryMessage,
        content: Option<&Value>,
    ) -> Result<(), HistoryError> {
        if message.role == "assistant" {
            self.append_reasoning(message)?;
        }
        let Some(value) = content else {
            return Ok(());
        };
        let converted = text_by_role(value, &message.role)?;
        self.push_message(&message.role, converted);
        Ok(())
    }

    fn append_reasoning(&mut self, message: &HistoryMessage) -> Result<(), HistoryError> {
        let items = super::reasoning::reasoning_input_items(message, &mut self.reasoning_ids)?;
        self.input.extend(items);
        Ok(())
    }

    fn push_message(&mut self, role: &str, content: Value) {
        let mut item = Map::new();
        item.insert("type".to_owned(), Value::String("message".to_owned()));
        item.insert("role".to_owned(), Value::String(role.to_owned()));
        item.insert("content".to_owned(), content);
        self.input.push(Value::Object(item));
    }
}
