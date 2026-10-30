use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicMetadata {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    Refusal,
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Option<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<Option<Vec<Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<Option<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        citations: Option<Option<Vec<Value>>>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    ToolUse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Option<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<Option<String>>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Thinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<Option<String>>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    RedactedThinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Option<String>>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Compaction {
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicMessagesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Option<Vec<ContentBlock>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<Option<StopReason>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_details: Option<Option<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub response_type: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Option<MessageUsage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_management: Option<Option<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safeguard_results: Option<Option<Vec<Value>>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedMessagesErrors {
    Error(super::AnthropicErrorResponse),
    Response(AnthropicMessagesResponse),
}

#[derive(Debug, thiserror::Error)]
pub enum WireDecodeError {
    #[error("messages JSON exceeds the configured limit")]
    TooLarge,
    #[error("messages JSON has too many items")]
    TooManyItems,
    #[error("messages JSON is not an error or response object")]
    Shape,
    #[error("messages JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn decode_messages_errors(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<DecodedMessagesErrors, WireDecodeError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(WireDecodeError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes)?;
    if count_items(&value) > limits.max_items {
        return Err(WireDecodeError::TooManyItems);
    }
    let object = value.as_object().ok_or(WireDecodeError::Shape)?;
    if object.get("type").and_then(Value::as_str) == Some("error") {
        return Ok(DecodedMessagesErrors::Error(serde_json::from_value(
            Value::Object(object.clone()),
        )?));
    }
    if object.get("type").and_then(Value::as_str) == Some("message") {
        return Ok(DecodedMessagesErrors::Response(serde_json::from_value(
            Value::Object(object.clone()),
        )?));
    }
    Err(WireDecodeError::Shape)
}

fn count_items(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(count_items).sum::<usize>(),
        Value::Object(fields) => 1 + fields.values().map(count_items).sum::<usize>(),
        _ => 1,
    }
}
