use std::borrow::Cow;

use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::records::{ChatChunk, ChunkFactory};
use super::translate::{delta, response_chunk};
use super::usage;
use crate::api::response_chat_history::metadata::{
    IncompleteFinish, finish_reason_for_incomplete, incomplete_reason, provider_metadata,
};
use crate::api::response_chat_history::reasoning::{
    ReasoningOutputItem, ReasoningRecord, chat_reasoning_items, reasoning_records,
};
use crate::api::responses_input_content::is_truthy;

pub(super) fn terminal_chunk(
    fields: &Map<String, Value>,
    event_type: Option<&str>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    let response_value = fields.get("response").unwrap_or(&Value::Null);
    let response = match response_value {
        Value::Null => Cow::Owned(Map::new()),
        Value::Object(payload) => Cow::Borrowed(payload),
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(ResponsesChatStreamError::Field { field: "response" });
        }
    };
    let output = match response.get("output") {
        Some(Value::Array(items)) => items.as_slice(),
        None | Some(Value::Null) => &[],
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "response.output",
            });
        }
    };
    let mut delta = delta(Some(""));
    delta.reasoning_items = reasoning_items(output)?;
    let mut chunk = response_chunk(
        factory,
        None,
        Some(0),
        delta,
        Some(finish_reason(response_value, event_type, output).to_owned()),
    );
    chunk.usage = Some(match response.get("usage") {
        Some(value) if is_truthy(Some(value)) => usage::chat_usage(value)?,
        None | Some(_) => Value::Null,
    });
    chunk.provider_specific_fields = terminal_metadata(&response);
    chunk.service_tier = terminal_service_tier(&response);
    Ok(chunk)
}

fn finish_reason(
    response_value: &Value,
    event_type: Option<&str>,
    output: &[Value],
) -> &'static str {
    if event_type == Some("response.incomplete") {
        return match finish_reason_for_incomplete(incomplete_reason(response_value).as_deref()) {
            IncompleteFinish::Length => "length",
            IncompleteFinish::ContentFilter => "content_filter",
        };
    }
    if output.iter().any(is_tool_call_item) {
        "tool_calls"
    } else {
        "stop"
    }
}

fn is_tool_call_item(item: &Value) -> bool {
    matches!(
        item,
        Value::Object(fields)
            if matches!(
                fields.get("type").and_then(Value::as_str),
                Some("function_call" | "custom_tool_call")
            )
    )
}

fn terminal_metadata(response: &Map<String, Value>) -> Option<Map<String, Value>> {
    let metadata = provider_metadata(Some(response));
    if metadata.is_empty() {
        return None;
    }
    Some(metadata)
}

fn terminal_service_tier(response: &Map<String, Value>) -> Option<Option<String>> {
    match response.get("service_tier") {
        Some(Value::String(tier)) => Some(Some(tier.clone())),
        None | Some(_) => None,
    }
}

// the terminal payload's output items carry reasoning records a chat client replays
fn reasoning_items(
    output: &[Value],
) -> Result<Option<Vec<ReasoningRecord>>, ResponsesChatStreamError> {
    let mut candidates = Vec::with_capacity(output.len());
    for item in output {
        candidates.push(match item {
            Value::Object(fields) => ReasoningOutputItem::Dict {
                value: fields.clone(),
            },
            Value::Array(_)
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Null => ReasoningOutputItem::TypedOther,
        });
    }
    Ok(chat_reasoning_items(&reasoning_records(&candidates)?))
}
