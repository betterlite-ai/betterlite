use std::convert::Infallible;

use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::{Extra, Json};
use indexmap::IndexMap;

use crate::converse_events::ConverseStreamState;
use crate::converse_events::error::ConverseEventError;
use crate::converse_events::events;
use crate::converse_events::response::{
    ConverseChunk, ConverseDelta, ThinkingBlock, ToolCallChunk,
};
use crate::converse_events::wire;
use crate::converse_usage::Usage;
use crate::converse_usage::error::UsageError;
use crate::converse_usage::reasoning::thinking_tokens_from_fields;
use crate::converse_usage::transform::{ReasoningTokenCounter, transform_usage};
use crate::converse_usage::wire::ConverseTokenUsage;

// The usage branch passes no reasoning content, so LiteLLM never asks the counter for an estimate.
struct UnusedCounter;

impl ReasoningTokenCounter for UnusedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}

pub(super) struct EventParts {
    pub(super) content: Option<String>,
    pub(super) tool_use: Option<ToolCallChunk>,
    pub(super) provider_fields: Option<Extra>,
    pub(super) reasoning_content: Option<String>,
    pub(super) thinking_blocks: Option<Vec<ThinkingBlock>>,
}

pub(super) struct ConverseProjection {
    pub(super) parts: EventParts,
    pub(super) finish_reason: Option<String>,
    pub(super) usage: Option<Usage>,
}

pub(super) fn project_converse(
    state: &mut ConverseStreamState,
    fields: &IndexMap<String, Json>,
    index: i64,
) -> Result<ConverseProjection, ConverseEventError> {
    let mut parts = EventParts {
        content: Some(String::new()),
        tool_use: None,
        provider_fields: None,
        reasoning_content: None,
        thinking_blocks: None,
    };
    let mut finish_reason = None;
    let mut usage = None;
    if let Some(start) = fields.get("start") {
        events::start_event(state, start, &mut parts)?;
    } else if let Some(delta) = fields.get("delta") {
        events::delta_event(state, delta, index, &mut parts)?;
    } else if fields.contains_key("contentBlockIndex") {
        parts.tool_use = events::stop_event(state, index)?;
    } else if let Some(reason) = fields.get("stopReason") {
        finish_reason = Some(mapped_finish_reason(reason));
        state.provider_reasoning_tokens = fields
            .get("additionalModelResponseFields")
            .and_then(thinking_tokens_from_fields);
    } else if fields.contains_key("usage") {
        usage = Some(usage_event(state, fields)?);
    } else {
        // an event with none of these keys still yields the empty delta LiteLLM builds
    }
    Ok(ConverseProjection {
        parts,
        finish_reason,
        usage,
    })
}

pub(super) fn assemble_converse(
    state: &ConverseStreamState,
    fields: &IndexMap<String, Json>,
    projection: ConverseProjection,
) -> ConverseChunk {
    let carries = carries_message_content(fields);
    let id = state
        .response_id
        .clone()
        .unwrap_or_else(|| format!("chatcmpl-{}", state.ids.next_id()));
    ConverseChunk {
        id,
        model: state.model.clone(),
        finish_reason: projection.finish_reason,
        index: 0,
        delta: ConverseDelta {
            content: if carries {
                projection.parts.content
            } else {
                None
            },
            role: carries.then(|| "assistant".to_owned()),
            tool_calls: projection.parts.tool_use.map(|call| vec![call]),
            provider_specific_fields: projection.parts.provider_fields,
            thinking_blocks: projection.parts.thinking_blocks,
            reasoning_content: projection.parts.reasoning_content,
        },
        usage: projection.usage,
        provider_specific_fields: trace_fields(fields),
    }
}

pub(super) fn initialize_response_id(
    state: &mut ConverseStreamState,
    fields: &IndexMap<String, Json>,
) -> Result<(), ConverseEventError> {
    if state.response_id.is_some() {
        return Ok(());
    }
    match fields.get("messageStart") {
        Some(message_start) => state.response_id = message_start_response_id(message_start)?,
        None => state.response_id = Some(format!("chatcmpl-{}", state.ids.next_id())),
    }
    Ok(())
}

fn message_start_response_id(value: &Json) -> Result<Option<String>, ConverseEventError> {
    let start = wire::object(value, "messageStart")?;
    let Some(conversation_id) = start.get("conversationId") else {
        return Ok(None);
    };
    match conversation_id {
        Json::Text(id) if !id.is_empty() => Ok(Some(format!("chatcmpl-{id}"))),
        Json::Text(_) | Json::Null => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ConverseEventError::FieldType {
                field: "messageStart.conversationId",
            })
        }
    }
}

fn usage_event(
    state: &ConverseStreamState,
    fields: &IndexMap<String, Json>,
) -> Result<Usage, ConverseEventError> {
    let value = fields
        .get("usage")
        .ok_or(ConverseEventError::Missing { field: "usage" })?;
    let block = ConverseTokenUsage::decode(value).map_err(UsageError::Usage)?;
    Ok(transform_usage(
        &block,
        None,
        state.thinking_ran,
        state.provider_reasoning_tokens,
        &UnusedCounter,
    )?)
}

fn carries_message_content(fields: &IndexMap<String, Json>) -> bool {
    fields.get("trace").is_some_and(wire::non_empty)
        || ["start", "delta", "contentBlockIndex", "stopReason"]
            .iter()
            .any(|key| fields.contains_key(*key))
}

fn trace_fields(fields: &IndexMap<String, Json>) -> Extra {
    let mut extra = Extra::new();
    let trace = fields
        .get("trace")
        .filter(|candidate| wire::non_empty(candidate));
    if let Some(value) = trace {
        extra.insert("trace".to_owned(), value.clone());
    }
    extra
}

fn mapped_finish_reason(reason: &Json) -> String {
    let raw = match reason {
        Json::Text(text) => text.as_str(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => "stop",
    };
    map_finish_reason(raw).reason.as_str().to_owned()
}
