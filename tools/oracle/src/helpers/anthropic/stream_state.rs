use betterlite_anthropic::stream_state::code::build_code_interpreter_results;
use betterlite_anthropic::stream_state::events::{
    ContentBlockStart, content_block_delta_helper, get_content_block_start,
    redacted_thinking_content,
};
use betterlite_anthropic::stream_state::json_fragment_buffer::AccumulatedJson as _;
use betterlite_anthropic::stream_state::message::{handle_message_delta, handle_usage};
use betterlite_anthropic::stream_state::state::{
    StreamOptions, StreamState, check_empty_tool_call_args,
};
use betterlite_anthropic::stream_state::web::{complete_web_search_call, web_search_call_snapshot};
use betterlite_anthropic::stream_state::wire::{ContentBlockDelta, JsonFields};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use deps::{FixedIds, TokenizerCounter};

mod deps;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    json_mode: Option<bool>,
    #[serde(default)]
    speed: Option<String>,
    #[serde(default)]
    sync_stream: bool,
    #[serde(default)]
    tool_name_reverse_map: IndexMap<String, String>,
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    Setup {
        #[serde(default)]
        fields: Box<StateFields>,
    },
    Delta {
        chunk: Json,
    },
    BlockStart {
        chunk: Json,
    },
    Redacted {
        chunk: Json,
    },
    CheckEmpty,
    Usage {
        usage: Json,
    },
    MessageDelta {
        chunk: Json,
    },
    WebSnapshot,
    CompleteWebSearch {
        result: Json,
    },
    CodeResults,
    State,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StateFields {
    content_blocks: Option<Vec<Json>>,
    web_search_calls: Option<IndexMap<String, Json>>,
    server_tool_inputs: Option<IndexMap<String, Json>>,
    tool_results: Option<Vec<Json>>,
    container_id: Option<String>,
    current_content_block_type: Option<String>,
    tool_index: Option<i64>,
    converted_response_format_tool: Option<bool>,
    accumulated_json: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let ids = FixedIds::parse(&input.ids)?;
        let mut state = StreamState::new(
            StreamOptions {
                sync_stream: input.sync_stream,
                json_mode: input.json_mode,
                speed: input.speed.clone(),
                tool_name_reverse_map: input.tool_name_reverse_map.clone(),
            },
            &ids,
        );
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            steps.push(step_value(&mut state, step).unwrap_or_else(|_| json!({ "failed": true })));
        }
        Ok(json!({ "steps": steps }))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StreamState".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn step_value(state: &mut StreamState, step: &Step) -> Result<Value, String> {
    match step {
        Step::Setup { fields } => {
            setup(state, fields)?;
            Ok(json!({ "state": state_view(state) }))
        }
        Step::CheckEmpty => Ok(json!({ "boolean": check_empty_tool_call_args(state) })),
        Step::State => Ok(json!({ "state": state_view(state) })),
        Step::Delta { chunk } => content_block_delta_helper(state, chunk)
            .map(|projection| json!({ "delta": projection }))
            .map_err(|error| error.to_string()),
        Step::BlockStart { chunk } => get_content_block_start(chunk)
            .map(|start| json!({ "start": start_view(&start) }))
            .map_err(|error| error.to_string()),
        Step::Redacted { chunk } => redacted_answer(chunk),
        Step::Usage { usage } => {
            let calculated =
                handle_usage(state, usage, &TokenizerCounter).map_err(|error| error.to_string())?;
            Ok(json!({
                "usage": calculated,
                "cumulative_usage": state.cumulative_usage,
            }))
        }
        Step::MessageDelta { chunk } => handle_message_delta(state, chunk, &TokenizerCounter)
            .map(|projection| json!({ "projection": projection }))
            .map_err(|error| error.to_string()),
        Step::WebSnapshot => Ok(json!({ "calls": web_search_call_snapshot(state) })),
        Step::CompleteWebSearch { result } => {
            complete_web_search_call(state, result);
            Ok(json!({ "calls": web_search_call_snapshot(state) }))
        }
        Step::CodeResults => build_code_interpreter_results(state)
            .map(|results| json!({ "results": results }))
            .map_err(|error| error.to_string()),
    }
}

fn redacted_answer(chunk: &Json) -> Result<Value, String> {
    let start = get_content_block_start(chunk).map_err(|error| error.to_string())?;
    let mut fields = JsonFields::new();
    let thinking_blocks =
        redacted_thinking_content(&start, &mut fields).map_err(|error| error.to_string())?;
    Ok(json!({
        "thinking_blocks": thinking_blocks,
        "provider_specific_fields": fields,
    }))
}

fn setup(state: &mut StreamState, fields: &StateFields) -> Result<(), String> {
    if let Some(content_blocks) = &fields.content_blocks {
        let mut blocks = Vec::with_capacity(content_blocks.len());
        for block in content_blocks {
            let text = serde_json::to_string(block).map_err(|error| error.to_string())?;
            let parsed = serde_json::from_str::<ContentBlockDelta>(&text)
                .map_err(|error| error.to_string())?;
            blocks.push(parsed);
        }
        state.content_blocks = blocks;
    }
    if let Some(calls) = &fields.web_search_calls {
        state.web_search_calls.clone_from(calls);
    }
    if let Some(inputs) = &fields.server_tool_inputs {
        state.server_tool_inputs.clone_from(inputs);
    }
    if let Some(results) = &fields.tool_results {
        state.tool_results.clone_from(results);
    }
    if let Some(container_id) = &fields.container_id {
        state.container_id = Some(container_id.clone());
    }
    if let Some(block_type) = &fields.current_content_block_type {
        state.current_content_block_type = Some(block_type.clone());
    }
    if let Some(index) = fields.tool_index {
        state.tool_index = index;
    }
    if let Some(converted) = fields.converted_response_format_tool {
        state.converted_response_format_tool = converted;
    }
    if let Some(value) = &fields.accumulated_json {
        state.json_buffer.set(value.clone());
    }
    Ok(())
}

fn state_view(state: &StreamState) -> Value {
    json!({
        "content_blocks": state.content_blocks,
        "tool_index": state.tool_index,
        "json_mode": state.json_mode,
        "speed": state.speed,
        "tool_name_reverse_map": state.tool_name_reverse_map,
        "response_id": state.response_id,
        "served_model": state.served_model,
        "is_response_format_tool": state.is_response_format_tool,
        "converted_response_format_tool": state.converted_response_format_tool,
        "chunk_type": state.chunk_type,
        "current_content_block_type": state.current_content_block_type,
        "web_search_results": state.web_search_results,
        "web_search_calls": state.web_search_calls,
        "compaction_blocks": state.compaction_blocks,
        "reasoning_content_chunks": state.reasoning_content_chunks,
        "server_tool_inputs": state.server_tool_inputs,
        "tool_results": state.tool_results,
        "current_server_tool_id": state.current_server_tool_id,
        "container_id": state.container_id,
        "accumulated_json": state.json_buffer.snapshot(),
    })
}

fn start_view(start: &ContentBlockStart) -> Value {
    json!({
        "index": start.index,
        "content_block": start.content_block,
    })
}
