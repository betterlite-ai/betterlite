use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::error::StreamStateError;
use super::state::StreamState;
use super::wire::{self, MessageBlockDelta};
use crate::usage::{AnthropicUsage, ReasoningTokenCounter, calculate_usage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageDeltaProjection {
    pub finish_reason: String,
    pub usage: Option<AnthropicUsage>,
    pub container: Option<Json>,
}

pub fn handle_usage<Counter>(
    state: &mut StreamState,
    usage: &Json,
    counter: &Counter,
) -> Result<AnthropicUsage, StreamStateError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let Json::Object(fields) = usage else {
        return Err(StreamStateError::FieldType { field: "usage" });
    };
    for (name, value) in fields {
        state.cumulative_usage.insert(name.clone(), value.clone());
    }
    let reasoning_content = if state.reasoning_content_chunks.is_empty() {
        None
    } else {
        Some(state.reasoning_content_chunks.concat())
    };
    let cumulative = Json::Object(state.cumulative_usage.clone());
    let calculated = calculate_usage(
        &cumulative,
        reasoning_content.as_deref(),
        None,
        state.speed.as_deref(),
        counter,
    )?;
    if let Some(speed) = calculated.speed.clone() {
        state.speed = Some(speed);
    }
    Ok(calculated)
}

pub fn handle_message_delta<Counter>(
    state: &mut StreamState,
    chunk: &Json,
    counter: &Counter,
) -> Result<MessageDeltaProjection, StreamStateError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let message = wire::decode::<MessageBlockDelta>(chunk)
        .map_err(|source| StreamStateError::Decode { source })?;
    let stop_reason = message
        .delta
        .stop_reason
        .as_deref()
        .filter(|reason| !reason.is_empty())
        .unwrap_or("stop");
    let finish_reason = if state.converted_response_format_tool {
        "stop"
    } else {
        map_finish_reason(stop_reason).reason.as_str()
    };
    let usage = match message.usage {
        Some(usage) => Some(handle_usage(state, &usage, counter)?),
        None => None,
    };
    Ok(MessageDeltaProjection {
        finish_reason: finish_reason.to_owned(),
        usage,
        container: message.delta.container,
    })
}
