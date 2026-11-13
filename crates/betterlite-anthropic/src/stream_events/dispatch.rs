use std::cmp::Ordering;

use betterlite_context::provider_message_primitives::FunctionKind;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::{
    SearchInput, SearchResult, WebSearchCall, WebSearchStatus, build_web_search_call,
};
use serde::Serialize;

use super::StreamEventConverter;
use super::chunk::{Branch, StreamEventToolCall};
use super::decode_stream_tool_arguments;
use super::error::{AnthropicError, ShapeError, StreamEventError};
use crate::stream_state::code::build_code_interpreter_results;
use crate::stream_state::events::{
    ContentBlockStart, ToolFunctionChunk, content_block_delta_helper, get_content_block_start,
    redacted_thinking_content,
};
use crate::stream_state::message::{handle_message_delta, handle_usage};
use crate::stream_state::state::{StreamState, check_empty_tool_call_args};
use crate::stream_state::web::{complete_web_search_call, web_search_call_snapshot};
use crate::stream_state::wire::{self, ContentBlockStartBody, JsonFields};
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

pub(crate) fn content_block_start<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    let start = get_content_block_start(chunk)?;
    let raw = content_block_json(chunk)?;
    converter.state.content_blocks.clear();
    converter.state.current_content_block_type = Some(start.content_block.kind.clone());
    match start.content_block.kind.as_str() {
        "text" => {
            start
                .content_block
                .text
                .as_ref()
                .ok_or(ShapeError::Missing {
                    field: "content_block.text",
                })?
                .clone_into(&mut branch.content);
        }
        "tool_use" | "server_tool_use" => start_tool(converter, &start, branch)?,
        "redacted_thinking" => {
            let mut fields = JsonFields::new();
            branch.thinking_blocks = Some(redacted_thinking_content(&start, &mut fields)?);
            branch.provider_specific_fields = fields;
        }
        "compaction" => start_compaction(converter, raw, branch),
        kind if kind.ends_with("_tool_result") => start_tool_result(converter, kind, raw, branch)?,
        _ => {}
    }
    Ok(())
}

fn content_block_json(chunk: &Json) -> Result<&Json, ShapeError> {
    match chunk {
        Json::Object(fields) => fields.get("content_block").ok_or(ShapeError::Missing {
            field: "content_block",
        }),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ShapeError::NotAnObject)
        }
    }
}

fn start_tool<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    start: &ContentBlockStart,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    let block = &start.content_block;
    let id = block.id.as_ref().ok_or(ShapeError::Missing {
        field: "content_block.id",
    })?;
    let index = converter
        .state
        .tool_index
        .checked_add(1)
        .ok_or(ShapeError::IndexOverflow)?;
    converter.state.tool_index = index;
    let original = block.name.as_ref().ok_or(ShapeError::Missing {
        field: "content_block.name",
    })?;
    let name = converter
        .state
        .tool_name_reverse_map
        .get(original)
        .map_or_else(|| original.to_owned(), Clone::clone);
    branch.tool_call = Some(StreamEventToolCall {
        id: Some(id.to_owned()),
        kind: FunctionKind::Function,
        function: ToolFunctionChunk {
            name: Some(name.clone()),
            arguments: String::new(),
        },
        index,
        caller: block.caller.clone().filter(has_value),
    });
    if block.kind == "server_tool_use" {
        start_server_tool(converter, block, id, &name, branch)?;
    }
    Ok(())
}

fn start_server_tool<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    block: &ContentBlockStartBody,
    id: &str,
    name: &str,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    converter.state.current_server_tool_id = Some(id.to_owned());
    let input = block
        .input
        .clone()
        .unwrap_or_else(|| Json::Object(JsonFields::new()));
    converter
        .state
        .server_tool_inputs
        .insert(id.to_owned(), input.clone());
    if name != "web_search" {
        return Ok(());
    }
    let empty_result = Json::Object(JsonFields::from([(
        "content".to_owned(),
        Json::List(Vec::new()),
    )]));
    let call = build_web_search_call(
        id,
        &wire::decode_field::<SearchInput>(&input).unwrap_or_default(),
        &wire::decode_field::<SearchResult>(&empty_result).unwrap_or_default(),
        Some(WebSearchStatus::InProgress),
    );
    store_web_search_call(converter, id, &call, branch)
}

fn store_web_search_call<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    id: &str,
    call: &WebSearchCall,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    converter
        .state
        .web_search_calls
        .insert(id.to_owned(), encode(call)?);
    branch.provider_specific_fields.insert(
        "web_search_calls".to_owned(),
        Json::Object(web_search_call_snapshot(&converter.state)),
    );
    Ok(())
}

fn start_compaction<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    raw: &Json,
    branch: &mut Branch,
) {
    converter.state.compaction_blocks.push(raw.clone());
    branch.provider_specific_fields.insert(
        "compaction_blocks".to_owned(),
        Json::List(converter.state.compaction_blocks.clone()),
    );
    let content = match raw {
        Json::Object(fields) => fields
            .get("content")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Json::Text(String::new())
        }
    };
    let mut start = JsonFields::new();
    start.insert("type".to_owned(), Json::Text("compaction".to_owned()));
    start.insert("content".to_owned(), content);
    branch
        .provider_specific_fields
        .insert("compaction_start".to_owned(), Json::Object(start));
}

fn start_tool_result<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    kind: &str,
    raw: &Json,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    match kind {
        "web_search_tool_result" => {
            converter.state.web_search_results.push(raw.clone());
            complete_web_search_call(&mut converter.state, raw);
            branch.provider_specific_fields.insert(
                "web_search_results".to_owned(),
                Json::List(converter.state.web_search_results.clone()),
            );
            branch.provider_specific_fields.insert(
                "web_search_calls".to_owned(),
                Json::Object(web_search_call_snapshot(&converter.state)),
            );
        }
        "web_fetch_tool_result" => {
            converter.state.web_search_results.push(raw.clone());
            branch.provider_specific_fields.insert(
                "web_search_results".to_owned(),
                Json::List(converter.state.web_search_results.clone()),
            );
        }
        "tool_search_tool_result" => {}
        _ => {
            converter.state.tool_results.push(raw.clone());
            branch.provider_specific_fields.insert(
                "tool_results".to_owned(),
                Json::List(converter.state.tool_results.clone()),
            );
            let results =
                build_code_interpreter_results(&converter.state).map_err(ShapeError::Event)?;
            branch
                .provider_specific_fields
                .insert("code_interpreter_results".to_owned(), encode(&results)?);
        }
    }
    Ok(())
}

pub(crate) fn content_block_stop<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), ShapeError> {
    stop_index(chunk)?;
    let block_type = converter.state.current_content_block_type.clone();
    let tool_block = matches!(block_type.as_deref(), Some("tool_use" | "server_tool_use"));
    if tool_block && check_empty_tool_call_args(&converter.state) {
        branch.tool_call = Some(StreamEventToolCall {
            id: None,
            kind: FunctionKind::Function,
            function: ToolFunctionChunk {
                name: None,
                arguments: "{}".to_owned(),
            },
            index: converter.state.tool_index,
            caller: None,
        });
    }
    if block_type.as_deref() == Some("server_tool_use")
        && let Some(id) = converter.state.current_server_tool_id.take()
    {
        let arguments = assembled_arguments(&converter.state);
        if let Ok(parsed) = decode_stream_tool_arguments(&arguments) {
            converter.state.server_tool_inputs.insert(id, parsed);
        }
    }
    converter.state.is_response_format_tool = false;
    converter.state.current_content_block_type = None;
    Ok(())
}

fn assembled_arguments(state: &StreamState) -> String {
    let mut arguments = String::new();
    for block in &state.content_blocks {
        if block.delta.kind == "input_json_delta"
            && let Some(partial) = &block.delta.partial_json
        {
            arguments.push_str(partial);
        }
    }
    arguments
}

fn stop_index(chunk: &Json) -> Result<(), ShapeError> {
    match chunk {
        Json::Object(fields) => match fields.get("index") {
            Some(Json::Number(_)) => Ok(()),
            Some(_) => Err(ShapeError::FieldType { field: "index" }),
            None => Err(ShapeError::Missing { field: "index" }),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ShapeError::NotAnObject)
        }
    }
}

pub(crate) fn message_delta<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let projection = handle_message_delta(&mut converter.state, chunk, &converter.counter)?;
    branch.finish_reason = Some(projection.finish_reason);
    branch.usage = projection.usage;
    let Some(container) = projection.container.filter(has_value) else {
        return Ok(());
    };
    branch
        .provider_specific_fields
        .insert("container".to_owned(), container.clone());
    let container_id = match &container {
        Json::Object(fields) => match fields.get("id") {
            Some(Json::Text(text)) if !text.is_empty() => Some(text.clone()),
            _ => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    let Some(stored_id) = container_id else {
        return Ok(());
    };
    if converter.state.tool_results.is_empty() {
        return Ok(());
    }
    converter.state.container_id = Some(stored_id);
    let results = build_code_interpreter_results(&converter.state).map_err(ShapeError::Event)?;
    branch
        .provider_specific_fields
        .insert("code_interpreter_results".to_owned(), encode(&results)?);
    Ok(())
}

pub(crate) fn message_start<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let fields = event_fields(chunk)?;
    let message = match fields.get("message") {
        Some(Json::Object(message)) => message,
        Some(_) => {
            return Err(StreamEventError::from(ShapeError::FieldType {
                field: "message",
            }));
        }
        None => {
            return Err(StreamEventError::from(ShapeError::Missing {
                field: "message",
            }));
        }
    };
    if let Some(model) = message.get("model") {
        converter.state.served_model = match model {
            Json::Null => None,
            Json::Text(text) => Some(text.clone()),
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(StreamEventError::from(ShapeError::FieldType {
                    field: "message.model",
                }));
            }
        };
    }
    if let Some(usage) = message.get("usage") {
        branch.usage = Some(handle_usage(
            &mut converter.state,
            usage,
            &converter.counter,
        )?);
    }
    Ok(())
}

pub(crate) fn native_error(chunk: &Json, fields: &JsonFields) -> AnthropicError {
    let message = match fields.get("error") {
        Some(Json::Object(error)) if !error.is_empty() => match error.get("message") {
            Some(Json::Text(message)) if !message.is_empty() => Some(message.clone()),
            _ => None,
        },
        _ => None,
    };
    AnthropicError {
        message: message.unwrap_or_else(|| serde_json::to_string(chunk).unwrap_or_default()),
        status_code: 500,
    }
}

fn has_value(value: &Json) -> bool {
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

fn encode<Value>(value: &Value) -> Result<Json, ShapeError>
where
    Value: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text).map_err(|source| ShapeError::Encode { source }),
        Err(source) => Err(ShapeError::Encode { source }),
    }
}
