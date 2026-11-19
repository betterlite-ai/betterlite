use std::fmt::Display;

use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::error::StreamFailure;
use betterlite_anthropic::anthropic_http_stream::transport::LineChunks;
use betterlite_anthropic::stream_decode::{ChunkStream, DecodedChunk};
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::text_completion_stream::chat_chunk;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::wrapper::{
    StreamSetup, StreamWrapper, WrapperError,
};
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue as StreamJson, StreamChunk, StreamOptions,
};
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use uuid::Uuid;

use crate::anthropic_dispatch::native::Counter;

// litellm.constants.REPEATED_CHUNK_LIMIT: the wrapper fails a model that repeats a chunk this many times.
const REPEATED_CHUNK_LIMIT: usize = 100;

struct Frames {
    stream: ChunkStream<LineChunks<StreamFailure>, Counter>,
    wrapper: StreamWrapper,
    model: String,
    include_usage: bool,
    done: bool,
}

pub(crate) fn frames(
    wrapped: WrappedStream<StreamFailure, Counter>,
    request: &ChatRequest,
    model: &str,
) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let include_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage);
    let setup = StreamSetup {
        model: model.to_owned(),
        stream_options: request
            .stream_options
            .as_ref()
            .map(|options| StreamOptions {
                include_usage: options.include_usage,
            }),
        merge_reasoning_in_choices: false,
        is_function_call: false,
        base_hidden_params: JsonObject::new(),
        repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
        now: system_instant,
        next_id: Uuid::new_v4,
    };
    let frames = Frames {
        stream: wrapped.completion_stream,
        wrapper: StreamWrapper::new("anthropic", setup),
        model: model.to_owned(),
        include_usage,
        done: false,
    };
    stream::unfold(frames, |mut frames| async move {
        let item = next_frame(&mut frames).await;
        item.map(|item| (item, frames))
    })
    .boxed()
}

async fn next_frame(frames: &mut Frames) -> Option<Result<Vec<u8>, HttpError>> {
    loop {
        match step(frames).await {
            Step::Frame(bytes) => return Some(Ok(bytes)),
            Step::Fail(error) => return Some(Err(error)),
            Step::Retry => {}
            Step::End => return None,
        }
    }
}

enum Step {
    Frame(Vec<u8>),
    Retry,
    End,
    Fail(HttpError),
}

async fn step(frames: &mut Frames) -> Step {
    if frames.done {
        return Step::End;
    }
    let decoded = match frames.stream.next().await {
        Some(Ok(decoded)) => decoded,
        Some(Err(error)) => return failed(frames, &error, "the anthropic provider stream failed"),
        None => return ending(frames),
    };
    match pushed(frames, decoded) {
        Ok(Some(chunk)) => written(frames, &chunk),
        Ok(None) => Step::Retry,
        Err(error) => failed(
            frames,
            &error,
            "the anthropic stream chunk could not be shaped",
        ),
    }
}

fn pushed(frames: &mut Frames, decoded: DecodedChunk) -> Result<Option<StreamChunk>, FrameError> {
    let chunk = match decoded {
        DecodedChunk::Event(event) => ProviderChunk::Json(Box::new(event_json(&event)?)),
        DecodedChunk::Generic(chunk) => ProviderChunk::Generic(chunk),
    };
    frames.wrapper.push(&chunk).map_err(FrameError::Wrapper)
}

// The event is LiteLLM's ModelResponseStream dump; a null model would fail the shared chunk reader.
fn event_json(event: &StreamEventChunk) -> Result<StreamJson, FrameError> {
    let serialized = serde_json::to_value(event).map_err(FrameError::Json)?;
    let mut value: StreamJson = serde_json::from_value(serialized).map_err(FrameError::Json)?;
    if let StreamJson::Object(fields) = &mut value
        && fields
            .get("model")
            .is_none_or(|model| matches!(model, StreamJson::Null))
    {
        fields.insert(String::from("model"), StreamJson::String(String::new()));
    }
    Ok(value)
}

fn written(frames: &mut Frames, chunk: &StreamChunk) -> Step {
    let value = match chat_chunk(chunk, &frames.model, frames.include_usage) {
        Ok(Some(value)) => value,
        Ok(None) => return Step::Retry,
        Err(error) => {
            return failed(
                frames,
                &error,
                "the anthropic stream chunk could not be written",
            );
        }
    };
    match serde_json::to_string(&value) {
        Ok(text) => Step::Frame(format!("data: {text}\n\n").into_bytes()),
        Err(error) => failed(
            frames,
            &error,
            "the anthropic stream chunk could not be written",
        ),
    }
}

fn ending(frames: &mut Frames) -> Step {
    frames.done = true;
    match frames.wrapper.finish() {
        None => Step::End,
        Some(Ok(chunk)) => written(frames, &chunk),
        Some(Err(error)) => failed(frames, &error, "the anthropic stream could not end"),
    }
}

fn failed(frames: &mut Frames, error: &dyn Display, message: &str) -> Step {
    tracing::warn!(message = %error, "{}", message);
    frames.done = true;
    Step::Fail(HttpError::Timeout)
}

#[derive(Debug, thiserror::Error)]
enum FrameError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Wrapper(#[from] WrapperError),
}
