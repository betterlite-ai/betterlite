use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use super::fingerprint::stream_item_field;
use super::stream_chunk_events::anthropic_stream_chunk_events;
use super::{Object, StreamItem};

mod tests;

#[derive(Debug, Clone, PartialEq)]
pub enum OriginalResponse {
    Absent,
    Response(Object),
    Stream(Vec<StreamItem>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BlockedUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResponsesUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
}

fn token_count(value: Option<&GuardrailValue>) -> i64 {
    match value {
        Some(GuardrailValue::SignedInteger(count)) => *count,
        Some(GuardrailValue::UnsignedInteger(count)) => i64::try_from(*count).unwrap_or(i64::MAX),
        _ => 0,
    }
}

// A key that is present but null counts 0 and does not fall back to the other key.
fn usage_tokens(usage: Option<&Object>, key: &str, fallback_key: &str) -> i64 {
    let Some(fields) = usage else {
        return 0;
    };
    token_count(fields.get(key).or_else(|| fields.get(fallback_key)))
}

fn usage_object(original_response: &OriginalResponse) -> Option<&Object> {
    let OriginalResponse::Response(response) = original_response else {
        return None;
    };
    match response.get("usage") {
        Some(GuardrailValue::Object(usage)) => Some(usage),
        _ => None,
    }
}

#[must_use]
pub fn anthropic_stream_usage(items: &[StreamItem]) -> Option<BlockedUsage> {
    let mut usage = BlockedUsage {
        input_tokens: 0,
        output_tokens: 0,
    };
    let mut found_usage = false;
    for event in items.iter().flat_map(anthropic_stream_chunk_events) {
        let usage_value = match event.get("type") {
            Some(GuardrailValue::Text(kind)) if kind == "message_start" => {
                match event.get("message") {
                    Some(GuardrailValue::Object(message)) => message.get("usage"),
                    _ => None,
                }
            }
            Some(GuardrailValue::Text(kind)) if kind == "message_delta" => event.get("usage"),
            _ => None,
        };
        let Some(GuardrailValue::Object(usage_fields)) = usage_value else {
            continue;
        };
        if let Some(input_tokens) = usage_fields
            .get("input_tokens")
            .filter(|value| !matches!(value, GuardrailValue::Null))
        {
            usage.input_tokens = token_count(Some(input_tokens));
            found_usage = true;
        }
        if let Some(output_tokens) = usage_fields
            .get("output_tokens")
            .filter(|value| !matches!(value, GuardrailValue::Null))
        {
            usage.output_tokens = token_count(Some(output_tokens));
            found_usage = true;
        }
    }
    found_usage.then_some(usage)
}

#[must_use]
pub fn blocked_response_usage(original_response: &OriginalResponse) -> BlockedUsage {
    if let OriginalResponse::Stream(items) = original_response
        && let Some(stream_usage) = anthropic_stream_usage(items)
    {
        return stream_usage;
    }
    let usage = usage_object(original_response);
    BlockedUsage {
        input_tokens: usage_tokens(usage, "input_tokens", "prompt_tokens"),
        output_tokens: usage_tokens(usage, "output_tokens", "completion_tokens"),
    }
}

#[must_use]
pub fn blocked_responses_api_usage(original_response: &OriginalResponse) -> ResponsesUsage {
    let usage = usage_object(original_response);
    let input_tokens = usage_tokens(usage, "input_tokens", "prompt_tokens");
    let output_tokens = usage_tokens(usage, "output_tokens", "completion_tokens");
    let total_tokens = usage_tokens(usage, "total_tokens", "total_tokens");
    ResponsesUsage {
        input_tokens,
        output_tokens,
        total_tokens: if total_tokens == 0 {
            input_tokens.saturating_add(output_tokens)
        } else {
            total_tokens
        },
    }
}

#[must_use]
pub fn blocked_chat_stream_usage(original_response: &OriginalResponse) -> (i64, i64) {
    let OriginalResponse::Stream(items) = original_response else {
        let usage = blocked_response_usage(original_response);
        return (usage.input_tokens, usage.output_tokens);
    };
    let usage = match items
        .iter()
        .rev()
        .find_map(|item| stream_item_field(item, "usage"))
    {
        Some(GuardrailValue::Object(usage)) => Some(usage),
        _ => None,
    };
    (
        usage_tokens(usage, "prompt_tokens", "input_tokens"),
        usage_tokens(usage, "completion_tokens", "output_tokens"),
    )
}

#[must_use]
pub fn blocked_responses_stream_usage(original_response: &OriginalResponse) -> ResponsesUsage {
    let OriginalResponse::Stream(items) = original_response else {
        return blocked_responses_api_usage(original_response);
    };
    let completed = items.iter().rev().find_map(|item| {
        let finished = matches!(
            stream_item_field(item, "type"),
            Some(GuardrailValue::Text(kind)) if kind == "response.completed"
        );
        match stream_item_field(item, "response") {
            Some(GuardrailValue::Object(response)) if finished => Some(response),
            _ => None,
        }
    });
    let completed_response = completed.map_or(OriginalResponse::Absent, |response| {
        OriginalResponse::Response(response.clone())
    });
    blocked_responses_api_usage(&completed_response)
}
