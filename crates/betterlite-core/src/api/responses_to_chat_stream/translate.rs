use std::collections::BTreeMap;

use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::passthrough::passthrough_chunk;
use super::records::{CHUNK_OBJECT, ChatChoice, ChatChunk, ChatDelta, ChatToolCall, ChunkFactory};
use super::terminal::terminal_chunk;
use super::text::repr_value;
use super::tool_call;
use crate::api::response_id_codecs::response::decode_previous_response_id;
use crate::api::responses_input_content::is_truthy;

pub type ToolCallIndices = BTreeMap<i64, i64>;

pub fn translate_responses_chunk_to_openai_stream(
    parsed_chunk: &Value,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    if !is_truthy(Some(parsed_chunk)) {
        return Err(ResponsesChatStreamError::EmptyChunk);
    }
    let Value::Object(fields) = parsed_chunk else {
        return Err(ResponsesChatStreamError::InvalidChunkType {
            kind: json_type_name(parsed_chunk),
        });
    };
    let event_type = event_type(fields)?;
    if fields.get("object").and_then(Value::as_str) == Some(CHUNK_OBJECT)
        || (event_type.is_none() && listed_choices(fields))
    {
        return passthrough_chunk(fields, factory);
    }
    tracing::debug!(
        "Chat provider: Processing event type: {}",
        event_type.unwrap_or("None")
    );
    let branch = match event_type {
        Some("response.created") => Some(created_chunk(parsed_chunk, fields, factory)?),
        Some("response.output_item.added") => added_chunk(fields, tool_call_index_map, factory)?,
        Some(
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta",
        ) => Some(argument_delta_chunk(
            parsed_chunk,
            fields,
            tool_call_index_map,
            factory,
        )?),
        Some("response.output_item.done") => item_done_chunk(fields, tool_call_index_map, factory)?,
        Some("response.output_text.delta") => {
            Some(text_delta_chunk(parsed_chunk, fields, factory)?)
        }
        Some("response.reasoning_summary_text.delta") => reasoning_delta_chunk(fields, factory)?,
        Some("response.completed" | "response.incomplete") => {
            Some(terminal_chunk(fields, event_type, factory)?)
        }
        None => None,
        Some(_) => None,
    };
    if let Some(chunk) = branch {
        return Ok(chunk);
    }
    tracing::debug!(
        "Chat provider: Unhandled event type '{}', creating empty chunk",
        event_type.unwrap_or("None")
    );
    Ok(response_chunk(
        factory,
        None,
        Some(0),
        delta(Some("")),
        None,
    ))
}

// an enum member of ResponsesAPIStreamEvents arrives as its value, and anything else is still compared as text
fn event_type(fields: &Map<String, Value>) -> Result<Option<&str>, ResponsesChatStreamError> {
    match fields.get("type") {
        None => Ok(None),
        Some(Value::String(kind)) => Ok(Some(kind)),
        Some(Value::Null) => Ok(None),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "type" }),
    }
}

// `parsed_chunk.get("choices")` is the whole list in the missing-type branch, so an empty one never passes through
fn listed_choices(fields: &Map<String, Value>) -> bool {
    match fields.get("choices") {
        Some(Value::Array(items)) => !items.is_empty(),
        None
        | Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_),
        ) => false,
    }
}

pub(super) fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "list",
        Value::Bool(_) => "bool",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::Object(_) => "dict",
        Value::String(_) => "str",
        Value::Null => "NoneType",
    }
}

pub(super) fn delta(content: Option<&str>) -> ChatDelta {
    ChatDelta {
        content: content.map(str::to_owned),
        role: None,
        function_call: None,
        tool_calls: None,
        audio: None,
        reasoning_content: None,
        reasoning_items: None,
        extra: Map::new(),
    }
}

pub(super) fn response_chunk(
    factory: ChunkFactory,
    id: Option<String>,
    index: Option<i64>,
    delta: ChatDelta,
    finish_reason: Option<String>,
) -> ChatChunk {
    ChatChunk {
        id: id.unwrap_or_else(|| format!("{CHAT_COMPLETION_ID_PREFIX}{}", (factory.identifier)())),
        object: CHUNK_OBJECT,
        created: (factory.clock)(),
        model: None,
        system_fingerprint: None,
        choices: vec![ChatChoice {
            index,
            delta,
            finish_reason,
            logprobs: None,
            extra: Map::new(),
        }],
        usage: None,
        provider_specific_fields: None,
        service_tier: None,
        extra: Map::new(),
    }
}

fn created_chunk(
    parsed_chunk: &Value,
    fields: &Map<String, Value>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    tracing::debug!(
        "Chat provider: response.created -> {}",
        repr_value(parsed_chunk)
    );
    let id = response_id(fields)?;
    Ok(response_chunk(factory, id, Some(0), delta(Some("")), None))
}

fn response_id(fields: &Map<String, Value>) -> Result<Option<String>, ResponsesChatStreamError> {
    match fields.get("response") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(response)) if response.is_empty() => Ok(None),
        Some(Value::Object(response)) => match response.get("id") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(id)) => Ok(Some(decode_previous_response_id(id))),
            Some(_) => Err(ResponsesChatStreamError::Field {
                field: "response.id",
            }),
        },
        Some(_) => Err(ResponsesChatStreamError::Field { field: "response" }),
    }
}

fn item_fields(
    fields: &Map<String, Value>,
) -> Result<Option<&Map<String, Value>>, ResponsesChatStreamError> {
    match fields.get("item") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(item)) => Ok(Some(item)),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "item" }),
    }
}

pub(super) fn output_index(
    fields: &Map<String, Value>,
) -> Result<Option<i64>, ResponsesChatStreamError> {
    match fields.get("output_index") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            number
                .as_i64()
                .map(Some)
                .ok_or(ResponsesChatStreamError::Field {
                    field: "output_index",
                })
        }
        Some(_) => Err(ResponsesChatStreamError::Field {
            field: "output_index",
        }),
    }
}

pub(super) fn sequential_index(
    map: Option<&mut ToolCallIndices>,
    output_index: i64,
) -> Result<i64, ResponsesChatStreamError> {
    let Some(entries) = map else {
        return Ok(output_index);
    };
    if let Some(index) = entries.get(&output_index) {
        return Ok(*index);
    }
    let next = i64::try_from(entries.len())
        .map_err(|source| ResponsesChatStreamError::IndexCount { source })?;
    entries.insert(output_index, next);
    Ok(next)
}

fn added_chunk(
    fields: &Map<String, Value>,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let Some(item) = item_fields(fields)? else {
        return Ok(None);
    };
    if !matches!(
        item.get("type").and_then(Value::as_str),
        Some("function_call" | "custom_tool_call")
    ) {
        return Ok(None);
    }
    let output_index = output_index(fields)?.unwrap_or(0);
    let index = sequential_index(tool_call_index_map, output_index)?;
    let call = tool_call::added_tool_call(fields, item, index)?;
    let mut delta = delta(None);
    delta.tool_calls = Some(vec![call]);
    Ok(Some(response_chunk(factory, None, Some(0), delta, None)))
}

fn argument_delta_chunk(
    parsed_chunk: &Value,
    fields: &Map<String, Value>,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    let arguments = match fields.get("delta") {
        Some(Value::String(text)) if !text.is_empty() => text,
        Some(Value::String(_) | Value::Null) | None => {
            return Err(ResponsesChatStreamError::InvalidArgumentDelta {
                event: repr_value(parsed_chunk),
            });
        }
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "delta" }),
    };
    let output_index = output_index(fields)?.unwrap_or(0);
    let index = sequential_index(tool_call_index_map, output_index)?;
    let call = ChatToolCall {
        id: None,
        index: Some(index),
        kind: Some("function".to_owned()),
        function: Some(tool_call::function_value(None, arguments, None)?),
        custom: None,
        provider_specific_fields: None,
        extra: Map::new(),
    };
    let mut delta = delta(None);
    delta.tool_calls = Some(vec![call]);
    Ok(response_chunk(factory, None, Some(0), delta, None))
}

fn item_done_chunk(
    fields: &Map<String, Value>,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let Some(item) = item_fields(fields)? else {
        return Ok(None);
    };
    match item.get("type").and_then(Value::as_str) {
        Some("function_call" | "custom_tool_call") => {
            let output_index = output_index(fields)?.unwrap_or(0);
            let mut delta = delta(None);
            if tool_call_index_map.is_none() {
                delta.tool_calls = Some(vec![tool_call::done_tool_call(item, output_index)?]);
            }
            Ok(Some(response_chunk(factory, None, Some(0), delta, None)))
        }
        Some("message") => Ok(Some(response_chunk(
            factory,
            None,
            Some(0),
            delta(Some("")),
            None,
        ))),
        None | Some(_) => Ok(None),
    }
}

fn text_delta_chunk(
    parsed_chunk: &Value,
    fields: &Map<String, Value>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    match fields.get("delta") {
        Some(Value::String(text)) => Ok(response_chunk(
            factory,
            None,
            Some(0),
            delta(Some(text)),
            None,
        )),
        Some(Value::Null) | None => Err(ResponsesChatStreamError::InvalidTextDelta {
            event: repr_value(parsed_chunk),
        }),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "delta" }),
    }
}

fn reasoning_delta_chunk(
    fields: &Map<String, Value>,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let reasoning = match fields.get("delta") {
        Some(Value::String(text)) if !text.is_empty() => text,
        Some(Value::String(_) | Value::Null) | None => return Ok(None),
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "delta" }),
    };
    let index = match fields.get("summary_index") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => {
            Some(number.as_i64().ok_or(ResponsesChatStreamError::Field {
                field: "summary_index",
            })?)
        }
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "summary_index",
            });
        }
    };
    let mut delta = delta(None);
    delta.reasoning_content = Some(reasoning.to_owned());
    Ok(Some(response_chunk(factory, None, index, delta, None)))
}
