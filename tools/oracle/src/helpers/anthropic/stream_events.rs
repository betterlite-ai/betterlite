use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use deps::{FixedIds, TokenCountFailure, TokenizerCounter};

mod deps;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    json_mode: Option<bool>,
    #[serde(default)]
    tool_name_reverse_map: IndexMap<String, String>,
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    Chunk {
        chunk: Json,
    },
    Accumulated {
        fragment: String,
        #[serde(default)]
        is_final: bool,
    },
    State,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let state = StreamState::new(
            StreamOptions {
                sync_stream: false,
                json_mode: input.json_mode,
                speed: None,
                tool_name_reverse_map: input.tool_name_reverse_map.clone(),
            },
            &FixedIds::parse(&input.ids)?,
        );
        let mut converter = StreamEventConverter {
            state,
            counter: TokenizerCounter,
            json_could_close: false,
        };
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            steps.push(step_value(&mut converter, step));
        }
        Ok(json!({ "steps": steps }))
    }
}

type Converter = StreamEventConverter<TokenizerCounter>;

fn failure_value(error: &StreamEventError<TokenCountFailure>) -> Value {
    match error {
        StreamEventError::Anthropic(native) => json!({ "error": { "status": native.status_code } }),
        StreamEventError::Shape(_) | StreamEventError::State { .. } => json!({ "failed": true }),
    }
}

fn step_value(converter: &mut Converter, step: &Step) -> Value {
    match step {
        Step::Chunk { chunk } => match converter.chunk_parser(chunk) {
            Ok(parsed) => json!({ "chunk": serde_json::to_value(parsed).unwrap_or(Value::Null) }),
            Err(error) => failure_value(&error),
        },
        Step::Accumulated { fragment, is_final } => {
            match converter.handle_accumulated_json_chunk(fragment, *is_final) {
                Ok(Some(parsed)) => {
                    json!({ "accumulated": serde_json::to_value(parsed).unwrap_or(Value::Null) })
                }
                Ok(None) => json!({ "accumulated": Value::Null }),
                Err(error) => failure_value(&error),
            }
        }
        Step::State => json!({ "state": state_view(converter) }),
    }
}

fn state_view(converter: &Converter) -> Value {
    json!({
        "current_content_block_type": converter.state.current_content_block_type,
        "current_server_tool_id": converter.state.current_server_tool_id,
        "is_response_format_tool": converter.state.is_response_format_tool,
        "converted_response_format_tool": converter.state.converted_response_format_tool,
        "tool_index": converter.state.tool_index,
        "served_model": converter.state.served_model,
        "container_id": converter.state.container_id,
        "server_tool_inputs": converter.state.server_tool_inputs,
    })
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StreamEvents".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
