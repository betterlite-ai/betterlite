use std::convert::Infallible;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::ids::IdFunction;

pub(crate) struct ZeroCounter;

impl ReasoningTokenCounter for ZeroCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(crate) fn converter(
    sync_stream: bool,
    json_mode: Option<bool>,
    next_id: fn() -> Uuid,
) -> StreamEventConverter<ZeroCounter> {
    let ids = IdFunction { next: next_id };
    let state = StreamState::new(
        StreamOptions {
            sync_stream,
            json_mode,
            speed: None,
            tool_name_reverse_map: IndexMap::new(),
        },
        &ids,
    );
    StreamEventConverter {
        state,
        counter: ZeroCounter,
        json_could_close: false,
    }
}

pub(crate) fn chunk_parser(
    converter: &mut StreamEventConverter<ZeroCounter>,
    chunk: &Json,
) -> Result<StreamEventChunk, InvokeStreamError> {
    converter
        .chunk_parser(chunk)
        .map_err(InvokeStreamError::Anthropic)
}
