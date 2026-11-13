use std::vec::IntoIter;

use betterlite_anthropic::stream_decode::error::{DecodeError, ReceiveError};
use betterlite_anthropic::stream_decode::passthrough::{
    ConvertError, ConvertedChunk, convert_chunk_to_generic_chunk,
};
use betterlite_anthropic::stream_decode::{ChunkStream, DecodedChunk, TransportChunk};
use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_events::error::{AnthropicError, StreamEventError};
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use futures_util::StreamExt as _;
use futures_util::stream;
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
    ids: Vec<String>,
    steps: Vec<Step>,
    passthrough: Vec<PassthroughStep>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    Text { value: String },
    Bytes { value: Vec<u8> },
    ValueError { message: String },
    OtherError { message: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum PassthroughStep {
    Text { value: String },
    Bytes { value: Vec<u8> },
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HelperFailure {
    message: String,
}

type SourceItem = Result<TransportChunk, ReceiveError<HelperFailure>>;
type SyncSources = IntoIter<SourceItem>;
type Failure = DecodeError<HelperFailure, TokenCountFailure>;
type Conversion = Result<ConvertedChunk, ConvertError<TokenCountFailure>>;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        // The sections run in the order the Python adapter evaluates them, so a native error aborts at the same point.
        let sync = sync_section(input)?;
        let asynchronous = async_section(input)?;
        let passthrough = passthrough_section(input)?;
        Ok(json!({
            "sync": sync,
            "async": asynchronous,
            "passthrough": passthrough,
        }))
    }
}

fn converter(ids: &[String]) -> Result<StreamEventConverter<TokenizerCounter>, Box<HelperError>> {
    let state = StreamState::new(
        StreamOptions {
            sync_stream: false,
            json_mode: None,
            speed: None,
            tool_name_reverse_map: IndexMap::new(),
        },
        &FixedIds::parse(ids)?,
    );
    Ok(StreamEventConverter {
        state,
        counter: TokenizerCounter,
        json_could_close: false,
    })
}

fn source_item(step: &Step) -> SourceItem {
    match step {
        Step::Text { value } => Ok(TransportChunk::Text(value.clone())),
        Step::Bytes { value } => Ok(TransportChunk::Bytes(value.clone())),
        Step::ValueError { message } => Err(ReceiveError::Value {
            source: HelperFailure {
                message: message.clone(),
            },
        }),
        Step::OtherError { message } => Err(ReceiveError::Other {
            source: HelperFailure {
                message: message.clone(),
            },
        }),
    }
}

fn sources(input: &Input) -> SyncSources {
    input
        .steps
        .iter()
        .map(source_item)
        .collect::<Vec<SourceItem>>()
        .into_iter()
}

fn sync_section(input: &Input) -> Result<Value, Box<HelperError>> {
    let (items, errors) = sync_outcomes(input)?;
    Ok(json!({ "items": items, "errors": errors }))
}

fn sync_outcomes(input: &Input) -> Result<(Vec<Value>, usize), Box<HelperError>> {
    let stream = ChunkStream {
        converter: converter(&input.ids)?,
        source: sources(input),
        source_ended: false,
    };
    record_outcomes(stream)
}

fn record_outcomes<I>(outcomes: I) -> Result<(Vec<Value>, usize), Box<HelperError>>
where
    I: IntoIterator<Item = Result<DecodedChunk, Failure>>,
{
    outcomes
        .into_iter()
        .try_fold((Vec::new(), 0), |(mut items, errors), outcome| {
            let errors = match outcome {
                Ok(chunk) => {
                    items.push(decoded_value(&chunk));
                    errors
                }
                Err(error) => errors.saturating_add(count_error(&error)?),
            };
            Ok::<(Vec<Value>, usize), Box<HelperError>>((items, errors))
        })
}

fn async_section(input: &Input) -> Result<Value, Box<HelperError>> {
    let (items, errors) = async_outcomes(input)?;
    Ok(json!({ "items": items, "errors": errors }))
}

fn async_outcomes(input: &Input) -> Result<(Vec<Value>, usize), Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    let chunks = stream::iter(sources(input));
    let mut stream = ChunkStream {
        converter: converter(&input.ids)?,
        source: chunks,
        source_ended: false,
    };
    let outcomes = runtime.block_on(async {
        let mut outcomes = Vec::new();
        while let Some(outcome) = stream.next().await {
            outcomes.push(outcome);
        }
        outcomes
    });
    record_outcomes(outcomes)
}

fn passthrough_section(input: &Input) -> Result<Value, Box<HelperError>> {
    let conversions = passthrough_outcomes(input)?;
    let (items, errors) = classify(conversions);
    Ok(json!({ "items": items, "errors": errors }))
}

fn passthrough_outcomes(input: &Input) -> Result<Vec<Conversion>, Box<HelperError>> {
    let mut decoder = converter(&input.ids)?;
    let mut conversions = Vec::new();
    for step in &input.passthrough {
        let conversion = convert_chunk_to_generic_chunk(&mut decoder, passthrough_chunk(step));
        if let Err(ConvertError::Event(StreamEventError::Anthropic(native))) = &conversion {
            return Err(native_error(native));
        }
        conversions.push(conversion);
    }
    Ok(conversions)
}

fn classify(conversions: Vec<Conversion>) -> (Vec<Value>, usize) {
    let mut items = Vec::new();
    let mut errors = 0;
    for conversion in conversions {
        match conversion {
            Ok(ConvertedChunk::Event(event)) => {
                items.push(json!({ "event": event_value(&event) }));
            }
            Ok(ConvertedChunk::Default { id }) => items.push(json!({ "default": id })),
            Err(_) => errors = errors.saturating_add(1),
        }
    }
    (items, errors)
}

fn passthrough_chunk(step: &PassthroughStep) -> TransportChunk {
    match step {
        PassthroughStep::Text { value } => TransportChunk::Text(value.clone()),
        PassthroughStep::Bytes { value } => TransportChunk::Bytes(value.clone()),
    }
}

// A native error leaves run() in Python, so the adapter reports it with its status instead of counting it.
fn count_error(error: &Failure) -> Result<usize, Box<HelperError>> {
    match error {
        DecodeError::Event(StreamEventError::Anthropic(native)) => Err(native_error(native)),
        DecodeError::Event(_)
        | DecodeError::Receiving { .. }
        | DecodeError::Other { .. }
        | DecodeError::Parsing { .. } => Ok(1),
    }
}

fn native_error(native: &AnthropicError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicError".to_owned(),
        status: Some(native.status_code),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: native.message.clone(),
    })
}

fn decoded_value(chunk: &DecodedChunk) -> Value {
    match chunk {
        DecodedChunk::Event(event) => json!({ "event": event_value(event) }),
        DecodedChunk::Generic(generic) => json!({ "generic": generic_value(generic) }),
    }
}

fn event_value(chunk: &StreamEventChunk) -> Value {
    let choice = chunk.choices.first();
    let delta = choice.map(|choice| &choice.delta);
    json!({
        "id": &chunk.id,
        "model": &chunk.model,
        "usage": &chunk.usage,
        "content": delta.map_or_else(String::new, |delta| delta.content.clone()),
        "finish_reason": choice.and_then(|choice| choice.finish_reason.clone()),
        "tool_calls": delta.and_then(|delta| delta.tool_calls.as_ref()),
        "provider_specific_fields": delta.and_then(|delta| delta.provider_specific_fields.as_ref()),
    })
}

fn generic_value(chunk: &GenericChunk) -> Value {
    json!({
        "text": &chunk.text,
        "tool_use": &chunk.tool_use,
        "is_finished": chunk.is_finished,
        "finish_reason": &chunk.finish_reason,
        "usage": &chunk.usage,
        "index": chunk.index,
    })
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StreamDecode".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
