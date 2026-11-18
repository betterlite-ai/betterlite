use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_anthropic::stream_state::state::{
    StreamOptions as EventOptions, StreamState, UuidV4,
};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_chunk_shaping::records::JsonValue;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage as WireUsage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::records::{Choice, Chunk};
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message as CountMessage;
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::gemini_stream::chunk::{Choice as StreamItem, StreamChunk};
use betterlite_vertex::gemini_stream::error::StreamError;
use betterlite_vertex::gemini_stream::{GeminiStreamDecoder, StreamOptions as GeminiOptions};
use http::HeaderMap;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::runtime::{Builder, Handle};
use tokio::task::block_in_place;
use uuid::Uuid;

#[derive(Debug, Error)]
pub(crate) enum CollectedStreamError {
    #[error("the collected Gemini chunk could not be decoded: {source}")]
    Decode {
        #[from]
        source: StreamError,
    },
    #[error("the collected Anthropic event could not be decoded: {source}")]
    Events {
        #[from]
        source: StreamEventError,
    },
    #[error("the collected chunk could not be projected: {0}")]
    Projection(#[from] serde_json::Error),
    #[error("the collected chunks could not be assembled: {source}")]
    Assemble {
        #[from]
        source: AssemblyError,
    },
    #[error("the log redactor could not be built")]
    Redaction {
        #[from]
        source: RedactionError,
    },
    #[error("the stream assembler runtime could not be started: {source}")]
    Runtime {
        #[source]
        source: std::io::Error,
    },
}

// One fresh decoder per call, one push per line and no finish: what is still open at the end is lost.
pub(crate) fn gemini_chunks(
    lines: &[String],
    now: i64,
) -> Result<Vec<Chunk>, CollectedStreamError> {
    let options = GeminiOptions {
        now,
        call_mode: CallMode::Tools,
    };
    let mut decoder = GeminiStreamDecoder::new(options, &HeaderMap::new(), Box::new(Uuid::new_v4));
    let mut chunks = Vec::with_capacity(lines.len());
    for line in lines {
        let Some(chunk) = decoder.push(line.as_bytes())? else {
            continue;
        };
        chunks.push(gemini_chunk(chunk)?);
    }
    Ok(chunks)
}

// The served model stays absent when the stream carries no modelVersion, like the SDK chunk LiteLLM sends on.
fn gemini_chunk(chunk: StreamChunk) -> Result<Chunk, CollectedStreamError> {
    let mut choices = Vec::with_capacity(chunk.choices.len());
    for item in chunk.choices {
        choices.push(match item {
            StreamItem::Projected(projected) => serde_json::from_value(
                serde_json::to_value(projected.as_ref()).map_err(CollectedStreamError::from)?,
            )?,
            StreamItem::EmptyDelta { index } => {
                let mut choice = Choice::default();
                choice.index = Some(index);
                choice
            }
        });
    }
    Ok(Chunk {
        hidden_params: None,
        id: Some(chunk.id),
        object: Some("chat.completion.chunk".to_owned()),
        created: Some(chunk.created),
        model: chunk.model,
        system_fingerprint: None,
        service_tier: chunk.service_tier,
        usage: chunk
            .usage
            .as_ref()
            .map(|usage| {
                let value = serde_json::to_value(super::reply::usage_json(usage))?;
                Ok(serde_json::from_value(value)?)
            })
            .transpose()?,
        choices,
    })
}

// LiteLLM hands stream_chunk_builder the SDK chunks, so one Anthropic event becomes one OpenAI chunk.
pub(crate) fn event_chunk(
    chunk: &StreamEventChunk,
    now: i64,
) -> Result<Chunk, CollectedStreamError> {
    let choices = serde_json::to_value(&chunk.choices)?;
    Ok(Chunk {
        hidden_params: None,
        id: Some(chunk.id.clone()),
        object: Some("chat.completion.chunk".to_owned()),
        created: Some(now),
        model: chunk.model.clone(),
        system_fingerprint: None,
        service_tier: None,
        usage: chunk
            .usage
            .as_ref()
            .map(|usage| serde_json::from_value(serde_json::to_value(usage)?))
            .transpose()?,
        choices: serde_json::from_value(choices)?,
    })
}

// LiteLLM builds the logged response with no logging object and no tokenizer in reach, so both stay inert.
pub(crate) fn complete_response(
    mut chunks: Vec<Chunk>,
    model: &str,
    now: i64,
) -> Result<Option<Json>, CollectedStreamError> {
    if chunks.is_empty() {
        return Ok(None);
    }
    // B2227 needs a model on the first chunk; LiteLLM's builder accepts a stream without one and logs no model at all.
    let named = chunks.iter().any(|chunk| chunk.model.is_some());
    for chunk in &mut chunks {
        if chunk.model.is_none() {
            chunk.model = Some(model.to_owned());
        }
    }
    let counter = ZeroUsage;
    let costs = NoCosts;
    let request = Request {
        chunks: Some(&chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: &costs,
        counter: &counter,
        metadata: None,
        now,
    };
    let hooks = silent_hooks()?;
    let built = block_on(stream_chunk_builder(&request, &hooks))??;
    let Some(response) = built else {
        return Ok(None);
    };
    let mut body: Json = serde_json::from_value(serde_json::to_value(&response)?)?;
    if !named && let Json::Object(fields) = &mut body {
        fields.insert("model".to_owned(), Json::Null);
    }
    Ok(Some(body))
}

// The handler is synchronous: a blocking task or a plain thread drives the assembler, never an async worker.
fn block_on<F: Future>(future: F) -> Result<F::Output, CollectedStreamError> {
    match Handle::try_current() {
        Ok(handle) => Ok(block_in_place(|| handle.block_on(future))),
        Err(_outside) => {
            let runtime = Builder::new_current_thread()
                .build()
                .map_err(|source| CollectedStreamError::Runtime { source })?;
            Ok(runtime.block_on(future))
        }
    }
}

pub(crate) struct ZeroReasoning;

impl ReasoningTokenCounter for ZeroReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

struct ZeroUsage;

#[async_trait]
impl UsageTokenCounter for ZeroUsage {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[CountMessage]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

struct NoCosts;

impl StreamCostCalculator for NoCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &WireUsage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        true
    }

    fn error(&self, _payload: &Payload) -> bool {
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

fn silent_hooks() -> Result<VerboseHooks<SilentLogger, SilentConsole>, CollectedStreamError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    Ok(VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

// The Anthropic per-stream state is public in full and starts from its defaults.
pub(crate) fn event_converter() -> StreamEventConverter<ZeroReasoning> {
    StreamEventConverter {
        state: StreamState::new(EventOptions::default(), &UuidV4),
        counter: ZeroReasoning,
        json_could_close: false,
    }
}
