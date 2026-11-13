use std::cmp::Ordering;

use betterlite_models::decisions::Json;
use serde::Serialize;

use super::StreamEventConverter;
use super::chunk::{Branch, StreamEventToolCall};
use super::error::{ShapeError, StreamEventError};
use crate::stream_state::events::content_block_delta_helper;
use crate::stream_state::wire::JsonFields;
use crate::usage::ReasoningTokenCounter;

pub(crate) const fn event_fields(chunk: &Json) -> Result<&JsonFields, ShapeError> {
    match chunk {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ShapeError::NotAnObject)
        }
    }
}

pub(crate) fn event_type(fields: &JsonFields) -> &str {
    match fields.get("type") {
        Some(Json::Text(kind)) => kind,
        _ => "",
    }
}

pub(crate) fn content_block_delta<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let projection =
        content_block_delta_helper(&mut converter.state, chunk).map_err(ShapeError::Event)?;
    branch.content = projection.text;
    branch.tool_call = projection.tool_use.map(|call| StreamEventToolCall {
        id: call.id,
        kind: call.kind,
        function: call.function,
        index: call.index,
        caller: None,
    });
    branch.thinking_blocks =
        (!projection.thinking_blocks.is_empty()).then_some(projection.thinking_blocks);
    branch.provider_specific_fields = projection.provider_specific_fields;
    branch.reasoning_content = projection.reasoning_content;
    Ok(())
}

pub(super) fn has_value(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_i64().map_or_else(
            || {
                number
                    .as_f64()
                    .is_some_and(|float| float.partial_cmp(&0.0_f64) != Some(Ordering::Equal))
            },
            |integer| integer != 0,
        ),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

pub(super) fn encode<Value>(value: &Value) -> Result<Json, ShapeError>
where
    Value: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text).map_err(|source| ShapeError::Encode { source }),
        Err(source) => Err(ShapeError::Encode { source }),
    }
}
