use std::convert::Infallible;
use std::io::Error as IoError;
use std::sync::Arc;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_state::state::{
    StreamOptions as EventOptions, StreamState, UuidV4,
};
use betterlite_anthropic::usage::error::UsageError;
use betterlite_anthropic::usage::{ReasoningTokenCounter, calculate_usage};
use betterlite_core::api::responses_to_chat_stream::records::CHUNK_OBJECT;
use betterlite_guardrails::repello_lifecycle::stream::{SilentConsole, SilentLogger};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::passthrough_reconstruction::converter::{
    EventChunkConverter, UsageCalculator,
};
use betterlite_streaming::passthrough_reconstruction::error::{ConvertError, ReconstructionError};
use betterlite_streaming::passthrough_reconstruction::events::{RawSseChunk, extract_sse_data};
use betterlite_streaming::passthrough_reconstruction::legacy::{AssemblyRequest, AssemblyServices};
use betterlite_streaming::passthrough_reconstruction::response::assemble_response;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage as WireUsage;
use betterlite_streaming::streamed_response_assembly::cost::{CostError, CostInput, StreamLogging};
use betterlite_streaming::streamed_response_assembly::records::Chunk as EventChunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::runtime;
use super::{Chunk, ConvertFailure, InterruptedStreamServices};
use crate::pass_through_logging::gemini_passthrough_logging::assembly::NoCosts;
use crate::pass_through_logging::gemini_passthrough_logging::counter::CollectedCounter;
use crate::pass_through_logging::response_costing::{
    ResponseCostError, ResponseCostService, compute_response_cost,
};

#[derive(Debug, Error)]
pub(super) enum AssemblyFailure {
    #[error("the buffered stream could not be reconstructed")]
    Reconstruction {
        #[source]
        source: Box<ReconstructionError<ConvertFailure>>,
    },
    #[error("the log redactor could not be built")]
    Redactor {
        #[source]
        source: Box<RedactionError>,
    },
    #[error("the bundled cl100k tokenizer could not be built")]
    Tokenizer,
    #[error("the stream assembler runtime could not be started")]
    Runtime {
        #[source]
        source: IoError,
    },
}

// LiteLLM hands stream_chunk_builder the logging object, whose cost calculator reads this call's log.
struct CostLogging<'state, Costs> {
    call_log: &'state LoggingState,
    model: &'state str,
    costs: &'state Costs,
}

impl<Costs> StreamLogging for CostLogging<'_, Costs>
where
    Costs: ResponseCostService,
{
    fn provider(&self) -> Option<&str> {
        match self.call_log.model_call_details.get("custom_llm_provider") {
            Some(Json::Text(provider)) => Some(provider.as_str()),
            None | Some(_) => None,
        }
    }

    fn response_cost(&self, input: &CostInput<'_>) -> Result<Option<Decimal>, CostError> {
        let response = cost_response(input)?;
        // LiteLLM's calculator swallows a pricing failure, while a router lookup failure escapes it.
        match compute_response_cost(self.costs, &response, self.model, self.call_log) {
            Ok(cost) => Ok(Some(cost)),
            Err(ResponseCostError::Completion(_failure)) => Ok(None),
            Err(error @ ResponseCostError::Router(_)) => Err(CostError {
                message: error.to_string(),
            }),
        }
    }
}

// The fields of the assembled response that completion_cost reads, as the JSON B3630's service takes.
fn cost_response(input: &CostInput<'_>) -> Result<Json, CostError> {
    let mut fields = IndexMap::new();
    fields.insert("model".to_owned(), Json::Text(input.model.to_owned()));
    let usage = serde_json::to_value(input.usage).map_err(|source| cost_failure(&source))?;
    fields.insert(
        "usage".to_owned(),
        serde_json::from_value(usage).map_err(|source| cost_failure(&source))?,
    );
    let hidden =
        serde_json::to_value(input.hidden_params).map_err(|source| cost_failure(&source))?;
    fields.insert(
        "_hidden_params".to_owned(),
        serde_json::from_value(hidden).map_err(|source| cost_failure(&source))?,
    );
    Ok(Json::Object(fields))
}

fn cost_failure(source: &serde_json::Error) -> CostError {
    CostError {
        message: source.to_string(),
    }
}

struct AnthropicChunkConverter {
    converter: StreamEventConverter<CollectedCounter>,
    now: i64,
}

impl EventChunkConverter for AnthropicChunkConverter {
    type Failure = ConvertFailure;

    fn convert_event(
        &mut self,
        event: &str,
    ) -> Result<Option<EventChunk>, ConvertError<Self::Failure>> {
        let Some(data) = extract_sse_data(event) else {
            return Ok(None);
        };
        let chunk = self
            .converter
            .chunk_parser(&data)
            .map_err(|source| ConvertError::Failed {
                source: ConvertFailure::from(source),
            })?;
        chunk_of(&chunk, self.now)
            .map(Some)
            .map_err(convert_failure)
    }
}

const fn convert_failure(source: serde_json::Error) -> ConvertError<ConvertFailure> {
    ConvertError::Failed {
        source: ConvertFailure::Json(source),
    }
}

// LiteLLM's ModelResponseStream stamps the object and creation time the builder reads.
fn chunk_of(chunk: &StreamEventChunk, now: i64) -> Result<EventChunk, serde_json::Error> {
    Ok(EventChunk {
        hidden_params: None,
        id: Some(chunk.id.clone()),
        object: Some(CHUNK_OBJECT.to_owned()),
        created: Some(now),
        model: chunk.model.clone(),
        system_fingerprint: None,
        service_tier: None,
        usage: serde_json::from_value(serde_json::to_value(&chunk.usage)?)?,
        choices: serde_json::from_value(serde_json::to_value(&chunk.choices)?)?,
    })
}

// LiteLLM's Anthropic usage reads no reasoning content, so the counter is never asked for a count.
struct ZeroReasoningCounter;

impl ReasoningTokenCounter for ZeroReasoningCounter {
    type Failure = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

#[derive(Debug, Error)]
pub(super) enum UsageBridgeError {
    #[error(transparent)]
    Usage(#[from] UsageError<Infallible>),
    #[error(transparent)]
    Convert(#[from] serde_json::Error),
}

// The builder asks for an OpenAI-shaped usage record; LiteLLM's Anthropic usage is mapped through JSON.
pub(super) struct AnthropicUsageCalculator;

impl UsageCalculator for AnthropicUsageCalculator {
    type Failure = UsageBridgeError;

    fn calculate_usage(
        &self,
        usage_object: &Json,
        speed: Option<&str>,
    ) -> Result<WireUsage, Self::Failure> {
        let usage = calculate_usage(usage_object, None, None, speed, &ZeroReasoningCounter)?;
        let value = serde_json::to_value(&usage)?;
        Ok(serde_json::from_value(value)?)
    }
}

pub(super) fn raw_chunk(chunk: &Chunk) -> RawSseChunk {
    match chunk {
        Chunk::Text(text) => RawSseChunk::Text(text.clone()),
        Chunk::Bytes(bytes) => RawSseChunk::Bytes(bytes.clone()),
    }
}

fn event_converter(
    speed: Option<&str>,
) -> Result<StreamEventConverter<CollectedCounter>, AssemblyFailure> {
    let counter = CollectedCounter::bundled().ok_or(AssemblyFailure::Tokenizer)?;
    let options = EventOptions {
        sync_stream: false,
        json_mode: None,
        speed: speed.map(str::to_owned),
        tool_name_reverse_map: IndexMap::new(),
    };
    Ok(StreamEventConverter {
        state: StreamState::new(options, &UuidV4),
        counter,
        json_could_close: false,
    })
}

fn silent_hooks() -> Result<VerboseHooks<SilentLogger, SilentConsole>, AssemblyFailure> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| AssemblyFailure::Redactor {
            source: Box::new(source),
        })?;
    Ok(VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

// LiteLLM's _build_complete_streaming_response, with the cost and usage seams the logging object holds.
pub(super) fn run<Costs, Counter>(
    all_chunks: &[Chunk],
    call_log: &LoggingState,
    model: &str,
    speed: Option<&str>,
    services: &InterruptedStreamServices<'_, Costs, Counter>,
) -> Result<Option<Response>, AssemblyFailure>
where
    Costs: ResponseCostService,
    Counter: UsageTokenCounter,
{
    let raw: Vec<RawSseChunk> = all_chunks.iter().map(raw_chunk).collect();
    let now = runtime::unix_seconds();
    let mut converter = AnthropicChunkConverter {
        converter: event_converter(speed)?,
        now,
    };
    let hooks = silent_hooks()?;
    let cost_logging = CostLogging {
        call_log,
        model,
        costs: services.costs,
    };
    let request = AssemblyRequest {
        chunks: &raw,
        converter: &mut converter,
        now,
    };
    let assembly_services = AssemblyServices {
        costs: &NoCosts,
        counter: services.counter,
        logging: Some(&cost_logging),
        hooks: &hooks,
    };
    // Any failure of the reconstruction is what LiteLLM's except turns into the usage-only fallback.
    let assembled = runtime::block_on(assemble_response(request, &assembly_services))
        .map_err(|source| AssemblyFailure::Runtime { source })?;
    assembled.map_err(|source| AssemblyFailure::Reconstruction {
        source: Box::new(source),
    })
}
