use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::records::{CHUNK_OBJECT, ChatChoice, ChatChunk, ChunkFactory};
use super::translate::delta;
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
    let absent = Map::new();
    let response = match response_value {
        Value::Null => &absent,
        Value::Object(payload) => payload,
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(ResponsesChatStreamError::Field { field: "response" });
        }
    };
    let output: &[Value] = match response.get("output") {
        Some(Value::Array(items)) => items.as_slice(),
        None | Some(Value::Null) => &[],
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "response.output",
            });
        }
    };
    let finish_reason = if event_type == Some("response.incomplete") {
        match finish_reason_for_incomplete(incomplete_reason(response_value).as_deref()) {
            IncompleteFinish::Length => "length",
            IncompleteFinish::ContentFilter => "content_filter",
        }
    } else if has_function_calls(output) {
        "tool_calls"
    } else {
        "stop"
    };
    let usage = match response.get("usage") {
        Some(value) if is_truthy(Some(value)) => Some(usage::chat_usage(value)?),
        None | Some(_) => None,
    };
    let metadata = provider_metadata(Some(response));
    let service_tier = match response.get("service_tier") {
        Some(Value::String(tier)) => Some(tier.clone()),
        None | Some(_) => None,
    };
    let records = output_reasoning_records(output)?;
    let mut delta = delta(Some(""));
    delta.reasoning_items = chat_reasoning_items(&records);
    Ok(ChatChunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", (factory.identifier)()),
        object: CHUNK_OBJECT,
        created: (factory.clock)(),
        model: None,
        system_fingerprint: None,
        choices: vec![ChatChoice {
            index: Some(0),
            delta,
            finish_reason: Some(finish_reason.to_owned()),
            logprobs: None,
            extra: Map::new(),
        }],
        usage: Some(usage.unwrap_or(Value::Null)),
        provider_specific_fields: if metadata.is_empty() {
            None
        } else {
            Some(metadata)
        },
        service_tier,
        extra: Map::new(),
    })
}

fn has_function_calls(items: &[Value]) -> bool {
    items.iter().any(|item| {
        matches!(
            item,
            Value::Object(fields)
                if matches!(
                    fields.get("type").and_then(Value::as_str),
                    Some("function_call" | "custom_tool_call")
                )
        )
    })
}

// the terminal payload's output items carry reasoning records a chat client replays
fn output_reasoning_records(
    output: &[Value],
) -> Result<Vec<ReasoningRecord>, ResponsesChatStreamError> {
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
    Ok(reasoning_records(&candidates)?)
}
