use std::task::Poll;
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
use futures_util::{StreamExt as _, stream};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError};

use deps::{FixedIds, TokenCountFailure, TokenizerCounter};

mod deps;
mod tests;

// The shared ApiLimits default max_event_bytes, so the helper decodes inside the same bound as production.
const MAX_EVENT_BYTES: usize = 0x0010_0000;

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
        let sync = sync_outcomes(input)?;
        let asynchronous = async_outcomes(input)?;
        let passthrough = classify(passthrough_outcomes(input)?)?;
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

fn sync_outcomes(input: &Input) -> Result<Vec<Value>, Box<HelperError>> {
    let stream = ChunkStream {
        converter: converter(&input.ids)?,
        source: sources(input),
        source_ended: false,
        max_event_bytes: MAX_EVENT_BYTES,
    };
    record_outcomes(stream)
}

fn record_outcomes<I>(outcomes: I) -> Result<Vec<Value>, Box<HelperError>>
where
    I: IntoIterator<Item = Result<DecodedChunk, Failure>>,
{
    outcomes
        .into_iter()
        .try_fold(Vec::new(), |mut recorded, outcome| {
            recorded.push(match outcome {
                Ok(chunk) => decoded_value(&chunk),
                Err(error) => error_label(&error)?,
            });
            Ok::<Vec<Value>, Box<HelperError>>(recorded)
        })
}

fn async_outcomes(input: &Input) -> Result<Vec<Value>, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_value_error()?;
    let mut items = sources(input);
    let mut pending = false;
    // A poll that waits before every item drives the stream through its wait path, as a real transport does.
    let chunks = stream::poll_fn(move |cx| {
        if pending {
            pending = false;
            return Poll::Ready(items.next());
        }
        pending = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    });
    let mut stream = ChunkStream {
        converter: converter(&input.ids)?,
        source: chunks,
        source_ended: false,
        max_event_bytes: MAX_EVENT_BYTES,
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

fn passthrough_outcomes(input: &Input) -> Result<Vec<Conversion>, Box<HelperError>> {
    let mut decoder = converter(&input.ids)?;
    let mut outcomes = Vec::with_capacity(input.passthrough.len());
    for step in &input.passthrough {
        outcomes.push(convert_chunk_to_generic_chunk(
            &mut decoder,
            passthrough_chunk(step),
        ));
    }
    Ok(outcomes)
}

fn classify(conversions: Vec<Conversion>) -> Result<Vec<Value>, Box<HelperError>> {
    conversions
        .into_iter()
        .try_fold(Vec::new(), |mut recorded, conversion| {
            recorded.push(match conversion {
                Ok(ConvertedChunk::Event(event)) => json!({ "event": event_value(&event) }),
                Ok(ConvertedChunk::Default { id }) => json!({ "default": id }),
                Err(ConvertError::Utf8(_)) => json!({ "error": "utf8" }),
                Err(ConvertError::Json(_)) => json!({ "error": "json" }),
                Err(ConvertError::Event(StreamEventError::Anthropic(native))) => {
                    return Err(native_error(&native));
                }
                Err(ConvertError::Event(_)) => json!({ "error": "event" }),
            });
            Ok::<Vec<Value>, Box<HelperError>>(recorded)
        })
}

fn passthrough_chunk(step: &PassthroughStep) -> TransportChunk {
    match step {
        PassthroughStep::Text { value } => TransportChunk::Text(value.clone()),
        PassthroughStep::Bytes { value } => TransportChunk::Bytes(value.clone()),
    }
}

// The labels match the adapter's: a native error leaves run() in Python, so the adapter reports it instead.
fn error_label(error: &Failure) -> Result<Value, Box<HelperError>> {
    let label = match error {
        DecodeError::Event(StreamEventError::Anthropic(native)) => {
            return Err(native_error(native));
        }
        DecodeError::Receiving { .. } => "receiving",
        DecodeError::Parsing { .. } => "parsing",
        DecodeError::Other { .. } => "other",
        DecodeError::Event(_) | DecodeError::TooLarge { .. } => "event",
    };
    Ok(json!({ "error": label }))
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
    let delta = choice.map(|selected| &selected.delta);
    json!({
        "id": &chunk.id,
        "model": &chunk.model,
        "usage": &chunk.usage,
        "content": delta.map_or_else(String::new, |part| part.content.clone()),
        "finish_reason": choice.and_then(|selected| selected.finish_reason.clone()),
        "tool_calls": delta.and_then(|part| part.tool_calls.as_ref()),
        "provider_specific_fields": delta.and_then(|part| part.provider_specific_fields.as_ref()),
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
