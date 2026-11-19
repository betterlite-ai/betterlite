use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState, UuidV4};
use betterlite_core::api::responses_to_chat_stream::records::CHUNK_OBJECT;
use betterlite_guardrails::repello_lifecycle::stream::{SilentConsole, SilentLogger};
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::passthrough_reconstruction::converter::EventChunkConverter;
use betterlite_streaming::passthrough_reconstruction::error::ConvertError;
use betterlite_streaming::passthrough_reconstruction::events::{RawSseChunk, extract_sse_data};
use betterlite_streaming::passthrough_reconstruction::legacy::{
    AssemblyRequest, AssemblyServices, assemble_response_legacy,
};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::{
    ChatMessage, ChatResponse, Response,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use serde_json::Value;

use super::error::{ConvertFailure, ProcessStreamError};
use crate::guardrails::anthropic_translation::sse_events::SseChunk;
use crate::guardrails::anthropic_translation::usage_response::{
    ZeroReasoningCounter, unix_seconds,
};

// The scan reads only the assembled text and tool calls, so the builder's token and cost seams stay silent.
struct SilentServices;

#[async_trait]
impl UsageTokenCounter for SilentServices {
    #[cfg_attr(test, mutants::skip)]
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

impl StreamCostCalculator for SilentServices {
    #[cfg_attr(test, mutants::skip)]
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    #[cfg_attr(test, mutants::skip)]
    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

// LiteLLM's splitter reads str and bytes chunks, so a stream's parsed event is rebuilt as its data line.
fn raw_chunk(chunk: &SseChunk) -> Result<RawSseChunk, ProcessStreamError> {
    match chunk {
        SseChunk::Bytes(bytes) => Ok(RawSseChunk::Bytes(bytes.clone())),
        SseChunk::Event(event) => {
            let payload = serde_json::to_string(event).map_err(ProcessStreamError::Convert)?;
            Ok(RawSseChunk::Text(format!("data: {payload}")))
        }
        SseChunk::Other => Ok(RawSseChunk::Text(String::new())),
    }
}

const fn convert_failure(source: serde_json::Error) -> ConvertError<ConvertFailure> {
    ConvertError::Failed {
        source: ConvertFailure::Json(source),
    }
}

struct AnthropicChunkConverter {
    converter: StreamEventConverter<ZeroReasoningCounter>,
    now: i64,
}

impl EventChunkConverter for AnthropicChunkConverter {
    type Failure = ConvertFailure;

    fn convert_event(&mut self, event: &str) -> Result<Option<Chunk>, ConvertError<Self::Failure>> {
        let Some(data) = extract_sse_data(event) else {
            return Ok(None);
        };
        let chunk = self
            .converter
            .chunk_parser(&data)
            .map_err(|source| ConvertError::Failed {
                source: ConvertFailure::from(source),
            })?;
        // LiteLLM's ModelResponseStream stamps the object and creation time its builder reads.
        let mut value = serde_json::to_value(&chunk).map_err(convert_failure)?;
        let Some(fields) = value.as_object_mut() else {
            return Ok(None);
        };
        fields.insert("object".to_owned(), Value::from(CHUNK_OBJECT));
        fields.insert("created".to_owned(), Value::from(self.now));
        serde_json::from_value(value)
            .map(Some)
            .map_err(convert_failure)
    }
}

#[cfg_attr(test, mutants::skip)]
fn silent_hooks() -> Result<VerboseHooks<SilentLogger, SilentConsole>, ProcessStreamError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    Ok(VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

fn anthropic_converter(now: i64) -> AnthropicChunkConverter {
    AnthropicChunkConverter {
        converter: StreamEventConverter {
            state: StreamState::new(StreamOptions::default(), &UuidV4),
            counter: ZeroReasoningCounter,
            json_could_close: false,
        },
        now,
    }
}

// LiteLLM's _build_complete_streaming_response over the buffered chunks, model empty; it returns None for an empty stream.
pub(super) async fn assemble_stream_response(
    chunks: &[SseChunk],
) -> Result<Option<Response>, ProcessStreamError> {
    let raw = chunks
        .iter()
        .map(raw_chunk)
        .collect::<Result<Vec<RawSseChunk>, ProcessStreamError>>()?;
    let now = unix_seconds();
    let hooks = silent_hooks()?;
    let costs = SilentServices;
    let counter = SilentServices;
    let mut converter = anthropic_converter(now);
    let request = AssemblyRequest {
        chunks: &raw,
        converter: &mut converter,
        now,
    };
    let services = AssemblyServices {
        costs: &costs,
        counter: &counter,
        logging: None,
        hooks: &hooks,
    };
    assemble_response_legacy(request, &services)
        .await
        .map_err(|source| ProcessStreamError::Assembly {
            source: Box::new(source),
        })
}

// LiteLLM's first_choice.message, reachable only for a chat response that carries a choice.
#[must_use]
pub(super) fn scanned_chat(response: Option<&Response>) -> Option<(&ChatResponse, &ChatMessage)> {
    let Response::Chat(chat) = response? else {
        return None;
    };
    let message = &chat.choices.first()?.message;
    Some((chat.as_ref(), message))
}
