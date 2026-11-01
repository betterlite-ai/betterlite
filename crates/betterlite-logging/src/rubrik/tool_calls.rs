use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatCompletionMessageToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Function,
}

#[derive(Clone, Debug)]
pub enum ToolCallSource {
    Normalized(ChatCompletionMessageToolCall),
    Raw(Json),
    Protocol {
        id: Option<String>,
        kind: Option<String>,
        function: Function,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolCallError {
    #[error("tool call has an unsupported shape")]
    UnsupportedShape,
}

pub fn normalize_tool_calls(
    tool_calls: &[ToolCallSource],
) -> Result<Vec<ChatCompletionMessageToolCall>, ToolCallError> {
    let mut normalized = Vec::with_capacity(tool_calls.len());
    for source in tool_calls {
        normalized.push(normalize_tool_call(source)?);
    }
    Ok(normalized)
}

pub fn normalize_tool_call(
    tool_call: &ToolCallSource,
) -> Result<ChatCompletionMessageToolCall, ToolCallError> {
    match tool_call {
        ToolCallSource::Normalized(call) => Ok(call.clone()),
        ToolCallSource::Raw(value) => normalize_raw(value),
        ToolCallSource::Protocol { id, kind, function } => {
            let protocol_kind = kind.clone().filter(|text| !text.is_empty());
            Ok(ChatCompletionMessageToolCall {
                id: id.clone().unwrap_or_default(),
                kind: protocol_kind.unwrap_or_else(|| "function".to_owned()),
                function: function.clone(),
            })
        }
    }
}

fn normalize_raw(value: &Json) -> Result<ChatCompletionMessageToolCall, ToolCallError> {
    let Json::Object(fields) = value else {
        return Err(ToolCallError::UnsupportedShape);
    };
    let function = match fields.get("function") {
        Some(Json::Object(function_fields)) => Function {
            name: text_field(function_fields, "name"),
            arguments: text_field(function_fields, "arguments"),
        },
        _ => Function {
            name: String::new(),
            arguments: String::new(),
        },
    };
    Ok(ChatCompletionMessageToolCall {
        id: text_field(fields, "id"),
        kind: match fields.get("type") {
            Some(Json::Text(text)) => text.clone(),
            _ => "function".to_owned(),
        },
        function,
    })
}

fn text_field(fields: &IndexMap<String, Json>, key: &str) -> String {
    match fields.get(key) {
        Some(Json::Text(text)) => text.clone(),
        _ => String::new(),
    }
}
