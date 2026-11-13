use std::collections::BTreeMap;

use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::fields::{Fields, field, json_type_name, listed_choices, sequential_index};
use super::passthrough::passthrough_chunk;
use super::records::{CHUNK_OBJECT, ChatChoice, ChatChunk, ChatDelta, ChatToolCall, ChunkFactory};
use super::terminal::terminal_chunk;
use super::text::repr_value;
use super::tool_call::{self, CallShape};
use crate::api::response_id_codecs::response::decode_previous_response_id;
use crate::api::responses_input_content::is_truthy;

pub type ToolCallIndices = BTreeMap<i64, i64>;

pub enum ItemEvent {
    Added,
    Done,
}

enum ContentKind {
    Created,
    Text,
}

pub fn translate_event(
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
    let event_type = field::<&str>(fields, "type")?;
    if is_chat_passthrough(fields, event_type) {
        return passthrough_chunk(fields, factory);
    }
    tracing::debug!(
        "Chat provider: Processing event type: {}",
        event_type.unwrap_or("None")
    );
    if let Some(chunk) = dispatch_event(
        parsed_chunk,
        fields,
        event_type,
        tool_call_index_map,
        factory,
    )? {
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

// an event already in chat format, or a payload with no type and a list of choices, is passed through whole
fn is_chat_passthrough(fields: &Fields, event_type: Option<&str>) -> bool {
    fields.get("object").and_then(Value::as_str) == Some(CHUNK_OBJECT)
        || (event_type.is_none() && listed_choices(fields))
}

fn dispatch_event(
    parsed_chunk: &Value,
    fields: &Fields,
    event_type: Option<&str>,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    match event_type {
        Some("response.created") => {
            content_chunk(parsed_chunk, fields, factory, ContentKind::Created).map(Some)
        }
        Some("response.output_text.delta") => {
            content_chunk(parsed_chunk, fields, factory, ContentKind::Text).map(Some)
        }
        Some("response.output_item.added") => {
            item_chunk(fields, tool_call_index_map, factory, ItemEvent::Added)
        }
        Some("response.output_item.done") => {
            item_chunk(fields, tool_call_index_map, factory, ItemEvent::Done)
        }
        Some(
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta",
        ) => argument_delta(parsed_chunk, fields, tool_call_index_map, factory),
        Some("response.reasoning_summary_text.delta") => reasoning_delta_chunk(fields, factory),
        Some("response.completed" | "response.incomplete") => {
            Ok(Some(terminal_chunk(fields, event_type, factory)?))
        }
        None | Some(_) => Ok(None),
    }
}

fn content_chunk(
    parsed_chunk: &Value,
    fields: &Fields,
    factory: ChunkFactory,
    kind: ContentKind,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    if matches!(kind, ContentKind::Text) {
        let Some(text) = field::<&str>(fields, "delta")? else {
            return Err(ResponsesChatStreamError::InvalidTextDelta {
                event: repr_value(parsed_chunk),
            });
        };
        return Ok(response_chunk(
            factory,
            None,
            Some(0),
            delta(Some(text)),
            None,
        ));
    }
    tracing::debug!(
        "Chat provider: response.created -> {}",
        repr_value(parsed_chunk)
    );
    let response = field::<&Fields>(fields, "response")?;
    let id = match response.and_then(|payload| payload.get("id")) {
        Some(Value::String(id)) => Some(decode_previous_response_id(id)),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "response.id",
            });
        }
    };
    Ok(response_chunk(factory, id, Some(0), delta(Some("")), None))
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

fn tool_chunk(factory: ChunkFactory, call: ChatToolCall) -> ChatChunk {
    let mut delta = delta(None);
    delta.tool_calls = Some(vec![call]);
    response_chunk(factory, None, Some(0), delta, None)
}

fn item_chunk(
    fields: &Fields,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
    event: ItemEvent,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let Some(item) = field::<&Fields>(fields, "item")? else {
        return Ok(None);
    };
    let item_type = item.get("type").and_then(Value::as_str);
    if item_type == Some("message") {
        return Ok(Some(response_chunk(
            factory,
            None,
            Some(0),
            delta(Some("")),
            None,
        )));
    }
    if !matches!(item_type, Some("function_call" | "custom_tool_call")) {
        return Ok(None);
    }
    let output_index = field::<i64>(fields, "output_index")?.unwrap_or(0);
    let shape = match event {
        ItemEvent::Added => {
            let index = sequential_index(tool_call_index_map, output_index)?;
            CallShape::Added {
                event: fields,
                index,
            }
        }
        ItemEvent::Done if tool_call_index_map.is_some() => {
            return Ok(Some(response_chunk(
                factory,
                None,
                Some(0),
                delta(None),
                None,
            )));
        }
        ItemEvent::Done => CallShape::Done {
            index: output_index,
        },
    };
    Ok(Some(tool_chunk(factory, tool_call::convert(item, shape)?)))
}

fn reasoning_delta_chunk(
    fields: &Fields,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let Some(reasoning) = field::<&str>(fields, "delta")?.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let index = field::<i64>(fields, "summary_index")?;
    let mut delta = delta(None);
    delta.reasoning_content = Some(reasoning.to_owned());
    Ok(Some(response_chunk(factory, None, index, delta, None)))
}

fn argument_delta(
    parsed_chunk: &Value,
    fields: &Fields,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
) -> Result<Option<ChatChunk>, ResponsesChatStreamError> {
    let arguments = argument_text(parsed_chunk, fields)?;
    Ok(Some(argument_chunk(
        fields,
        tool_call_index_map,
        factory,
        arguments,
    )?))
}

fn argument_text<'a>(
    parsed_chunk: &Value,
    fields: &'a Fields,
) -> Result<&'a str, ResponsesChatStreamError> {
    field::<&str>(fields, "delta")?
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ResponsesChatStreamError::InvalidArgumentDelta {
            event: repr_value(parsed_chunk),
        })
}

fn argument_chunk(
    fields: &Fields,
    tool_call_index_map: Option<&mut ToolCallIndices>,
    factory: ChunkFactory,
    arguments: &str,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    let output_index = field::<i64>(fields, "output_index")?.unwrap_or(0);
    let index = sequential_index(tool_call_index_map, output_index)?;
    let mut delta = delta(None);
    delta.tool_calls = Some(vec![tool_call::delta_call(index, arguments)?]);
    Ok(response_chunk(factory, None, Some(0), delta, None))
}
