use std::future::Future;
use std::sync::Arc;

use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_models::decisions::Json;
use betterlite_openai::cohere_streams::{CohereStreamVersion, decode_cohere_line};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject as HiddenParams,
};
use betterlite_streaming::streamed_response_assembly::records::Chunk as CollectedChunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};
use tokio::runtime::{Builder, Handle};
use tokio::task::block_in_place;
use uuid::Uuid;

use crate::pass_through_logging::gemini_passthrough_logging::assembly::{
    NoCosts, SilentConsole, SilentLogger,
};
use crate::pass_through_logging::gemini_passthrough_logging::counter::CollectedCounter;

use super::super::base_passthrough_logging::{PassthroughLoggingError, PassthroughStage};
use super::{COHERE, CohereLoggingError, unix_seconds};

// LiteLLM reads REPEATED_STREAMING_CHUNK_LIMIT's default, 100.
const REPEATED_CHUNK_LIMIT: usize = 100;

// LiteLLM wraps the V1 iterator in CustomStreamWrapper, then hands the kept chunks to stream_chunk_builder.
pub(crate) fn complete_streaming_response(
    all_chunks: &[String],
    model: &str,
) -> Result<Option<StreamedResponse>, PassthroughLoggingError> {
    let chunks = collected_chunks(all_chunks, model)?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let counter = CollectedCounter::bundled()
        .ok_or_else(|| PassthroughStage::StreamBuild.failed(CohereLoggingError::Tokenizer))?;
    let costs = NoCosts;
    let request = Request {
        chunks: Some(&chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: &costs,
        counter: &counter,
        metadata: None,
        now: i64::try_from(unix_seconds()).unwrap_or(i64::MAX),
    };
    let hooks = silent_hooks()?;
    let built = block_on(stream_chunk_builder(&request, &hooks))?
        .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
    match built {
        None => Ok(None),
        Some(Response::Chat(chat)) => {
            let chat_json = serde_json::to_string(chat.as_ref())
                .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
            let logged: Json = serde_json::from_str(&chat_json)
                .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
            Ok(Some(StreamedResponse::Model(logged)))
        }
        Some(Response::Text(text)) => {
            let text_json = serde_json::to_string(&text)
                .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
            let logged: Json = serde_json::from_str(&text_json)
                .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
            Ok(Some(StreamedResponse::TextCompletion(logged)))
        }
    }
}

// One generic chunk per line, shaped by a fresh wrapper state; StopIteration ends the collection.
fn collected_chunks(
    all_chunks: &[String],
    model: &str,
) -> Result<Vec<CollectedChunk>, PassthroughLoggingError> {
    let mut shaper = ChunkShaper::new(
        model.to_owned(),
        Some(COHERE.to_owned()),
        false,
        HiddenParams::new(),
    );
    let mut state = decode_state(model);
    let mut chunks = Vec::with_capacity(all_chunks.len());
    for line in all_chunks {
        let decoded = decode_cohere_line(CohereStreamVersion::V1, line.as_bytes(), false)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
        let decoded_json = serde_json::to_string(&decoded)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
        let generic: GenericChunk = serde_json::from_str(&decoded_json)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
        let chunk = ProviderChunk::Generic(Box::new(generic));
        let emitted = match chunk_creator(&mut shaper, &mut state, &chunk) {
            Ok(Some(kept)) => kept,
            Ok(None) => continue,
            Err(ProviderChunkError::StopIteration) => break,
            Err(error) => return Err(PassthroughStage::StreamBuild.failed(error)),
        };
        let emitted_json = serde_json::to_string(&emitted)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
        let collected: CollectedChunk = serde_json::from_str(&emitted_json)
            .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
        chunks.push(collected);
    }
    Ok(chunks)
}

// The wrapper state LiteLLM keeps across the chunks of one stream.
fn decode_state(model: &str) -> DecodeState {
    DecodeState {
        model: model.to_owned(),
        kind: ProviderKind::Builtin,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: None,
        options: RequestOptions {
            send_stream_usage: false,
            is_function_call: false,
        },
        repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
        next_id: Uuid::new_v4,
    }
}

// The stream builder has no logging object in this path, so its verbose output goes nowhere.
fn silent_hooks() -> Result<VerboseHooks<SilentLogger, SilentConsole>, PassthroughLoggingError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
    Ok(VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

// The handler is synchronous: a blocking task or a plain thread drives the assembler, never an async worker.
fn block_on<F>(future: F) -> Result<F::Output, PassthroughLoggingError>
where
    F: Future,
{
    match Handle::try_current() {
        Ok(handle) => Ok(block_in_place(|| handle.block_on(future))),
        Err(_outside) => {
            let runtime = Builder::new_current_thread()
                .build()
                .map_err(|source| PassthroughStage::StreamBuild.failed(source))?;
            Ok(runtime.block_on(future))
        }
    }
}
