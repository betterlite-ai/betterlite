use serde::Deserialize;
use serde_json::value::RawValue;

use super::error::CompletionError;
use crate::realtime_session::records::{Setting, nullable_setting};

#[derive(Debug, Deserialize)]
pub struct ToolUseEvent {
    #[serde(rename = "toolUse")]
    pub tool_use: ToolUse,
}

// The ids and the name are absent or null in Bedrock's toolUse and read as empty text
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ToolUse {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(rename = "toolName")]
    pub tool_name: Option<String>,
    #[serde(deserialize_with = "nullable_setting")]
    pub input: Option<Setting<Box<RawValue>>>,
}

// LiteLLM decodes a string input as JSON and keeps `{}` when that text is not JSON
pub fn decode_realtime_tool_input(bytes: &[u8]) -> Result<Box<RawValue>, CompletionError> {
    let raw: Box<RawValue> =
        serde_json::from_slice(bytes).map_err(|source| CompletionError::ToolInput { source })?;
    let Ok(text) = serde_json::from_str::<String>(raw.get()) else {
        return Ok(raw);
    };
    RawValue::from_string(text)
        .or_else(|_| RawValue::from_string("{}".to_owned()))
        .map_err(|source| CompletionError::ToolInput { source })
}
