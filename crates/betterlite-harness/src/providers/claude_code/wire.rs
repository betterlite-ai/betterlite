use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::StreamError;
use crate::payloads::python_json::python_json_text;
use crate::types::JsonObject;

#[path = "tool_json.rs"]
mod tool_json;

pub(super) const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum Line {
    #[serde(rename = "assistant")]
    Assistant(AssistantLine),
    #[serde(rename = "user")]
    User(UserLine),
    #[serde(rename = "system")]
    System(SystemLine),
    #[serde(rename = "result")]
    Result(ResultLine),
    #[serde(other)]
    Other,
}

fn decode<T>(text: Result<String, serde_json::Error>, kind: &str) -> Result<T, StreamError>
where
    T: DeserializeOwned,
{
    let invalid_shape = |source| StreamError::Shape {
        kind: kind.to_owned(),
        source,
    };
    let encoded = text.map_err(&invalid_shape)?;
    serde_json::from_str(&encoded).map_err(invalid_shape)
}

pub(super) fn parse(line: &JsonObject) -> Result<Line, StreamError> {
    let kind = line.get("type").and_then(|value| value.as_str());
    match kind {
        Some(name @ ("assistant" | "user" | "system" | "result")) => {
            decode(serde_json::to_string(line), name)
        }
        Some(_) | None => Ok(Line::Other),
    }
}

#[derive(Deserialize)]
pub(super) struct AssistantLine {
    #[serde(default)]
    pub(super) parent_tool_use_id: Option<String>,
    pub(super) message: AssistantMessage,
}

#[derive(Deserialize)]
pub(super) struct AssistantMessage {
    #[serde(default)]
    pub(super) model: Option<String>,
    pub(super) content: MessageContent,
}

#[derive(Deserialize)]
pub(super) struct UserLine {
    #[serde(default)]
    pub(super) parent_tool_use_id: Option<String>,
    pub(super) message: UserMessage,
}

#[derive(Deserialize)]
pub(super) struct UserMessage {
    pub(super) content: MessageContent,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum MessageContent {
    Text(String),
    Blocks(Vec<WireValue>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum Block {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Option<WireValue>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: WireValue,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum WireValue {
    #[default]
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

pub(super) fn stringify_tool_output(content: &WireValue) -> String {
    // LiteLLM leaves null empty, but serializes every other non-text value as JSON.
    match content {
        WireValue::Null => String::new(),
        WireValue::Text(text) => text.clone(),
        WireValue::Array(blocks) => blocks
            .iter()
            .map(stringify_tool_output_block)
            .collect::<Vec<_>>()
            .join("\n"),
        WireValue::Bool(_) | WireValue::Number(_) | WireValue::Object(_) => {
            tool_json::encode(content)
        }
    }
}

fn stringify_tool_output_block(block: &WireValue) -> String {
    if let WireValue::Text(text) = block {
        return text.clone();
    }
    if let WireValue::Object(fields) = block
        && matches!(fields.get("type"), Some(WireValue::Text(kind)) if kind == "text")
    {
        return match fields.get("text") {
            Some(WireValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
    }
    tool_json::encode(block)
}

pub(super) fn block(value: &WireValue) -> Result<Block, StreamError> {
    let WireValue::Object(fields) = value else {
        return Ok(Block::Other);
    };
    let Some(WireValue::Text(kind)) = fields.get("type") else {
        return Ok(Block::Other);
    };
    decode(python_json_text(value), kind)
}

pub(super) fn object(value: &WireValue) -> JsonObject {
    let text = python_json_text(&value).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

#[derive(Deserialize)]
pub(super) struct SystemLine {
    #[serde(default)]
    pub(super) subtype: Option<String>,
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) compact_metadata: Option<CompactMetadata>,
}

#[derive(Deserialize)]
pub(super) struct CompactMetadata {
    #[serde(default)]
    pub(super) pre_tokens: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct ResultLine {
    #[serde(default)]
    pub(super) is_error: bool,
    #[serde(default)]
    pub(super) result: Option<WireValue>,
    #[serde(default)]
    pub(super) errors: Option<Vec<String>>,
    #[serde(default)]
    pub(super) structured_output: Option<StructuredOutputField>,
    #[serde(default)]
    pub(super) session_id: Option<String>,
}

// state.structured_output is only set when the CLI's structured_output is a JSON object; see pitfalls
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum StructuredOutputField {
    Dict(JsonObject),
    Other(IgnoredAny),
}

impl StructuredOutputField {
    pub(super) fn into_dict(self) -> Option<JsonObject> {
        match self {
            Self::Dict(object) => Some(object),
            Self::Other(_) => None,
        }
    }
}
