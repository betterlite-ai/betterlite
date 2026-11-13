use betterlite_context::provider_message_primitives::FunctionKind;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::{
    SearchInput, SearchResult, WebSearchCall, WebSearchStatus, build_web_search_call,
};

use super::StreamEventConverter;
use super::chunk::{Branch, StreamEventToolCall};
use super::dispatch::{encode, has_value};
use super::error::ShapeError;
use crate::stream_state::code::build_code_interpreter_results;
use crate::stream_state::events::{
    ContentBlockStart, ToolFunctionChunk, get_content_block_start, redacted_thinking_content,
};
use crate::stream_state::web::{complete_web_search_call, web_search_call_snapshot};
use crate::stream_state::wire::{self, ContentBlockStartBody, JsonFields};

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
    let index = converter
        .state
        .tool_index
        .checked_add(1)
        .ok_or(ShapeError::IndexOverflow)?;
    converter.state.tool_index = index;
    let original = block.name.as_ref().ok_or(ShapeError::Missing {
        field: "content_block.name",
    })?;
    let id = block.id.as_ref().ok_or(ShapeError::Missing {
        field: "content_block.id",
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
