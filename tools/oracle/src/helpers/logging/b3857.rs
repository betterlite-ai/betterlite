mod assembler;
mod input;
mod record;
mod tests;

use betterlite_logging::otel::content_projections::{
    OutputResponse, RelayedChunk, request_input, response_output, stream_output,
};
use serde_json::Value;

use assembler::ChunkBuilderAssembler;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use input::{ChunkInput, ChunkKind, Function, Input, ResponseKind};
use record::{invalid, record_success};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::RecordSuccess => record_success(input),
            Function::RequestInput => parsed(request_input(&input.data)),
            Function::ResponseOutput => response_text(input),
            Function::StreamOutput => {
                let relayed = input
                    .chunks
                    .iter()
                    .map(relay)
                    .collect::<Result<Vec<_>, _>>()?;
                parsed(stream_output(&relayed, &input.data, &ChunkBuilderAssembler))
            }
        }
    }
}

// the projections return JSON text, and both sides read it back so the spacing never matters
fn parsed(text: Option<String>) -> HelperResult {
    text.map_or(Ok(Value::Null), |json| {
        serde_json::from_str(&json).map_err(|error| invalid(&error.to_string()))
    })
}

fn response_text(input: &Input) -> HelperResult {
    let response = input
        .response
        .as_ref()
        .ok_or_else(|| invalid("response is required"))?;
    let wrapped = match input.kind {
        Some(ResponseKind::ModelResponse) => OutputResponse::ModelResponse(response),
        Some(ResponseKind::ResponsesApi) => OutputResponse::ResponsesApi(response),
        Some(ResponseKind::Other) => OutputResponse::Other(response),
        None => return Err(invalid("kind is required")),
    };
    parsed(response_output(&wrapped))
}

// a typed completed chunk holds the whole event, and the port reads only its response
fn relay(chunk: &ChunkInput) -> Result<RelayedChunk, Box<HelperError>> {
    match chunk.kind {
        ChunkKind::Chat => Ok(RelayedChunk::Chat(chunk.value.clone())),
        ChunkKind::Other => Ok(RelayedChunk::Other(chunk.value.clone())),
        ChunkKind::Completed => chunk
            .value
            .as_object()
            .and_then(|event| event.get("response"))
            .map(|response| RelayedChunk::Completed(response.clone()))
            .ok_or_else(|| invalid("a completed chunk needs a response")),
    }
}
