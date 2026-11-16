use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::otel::content_projections::StreamOutputAssembler;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use tokio::runtime::Builder;

// the cases give every chunk stream a usage record, so nothing is counted or priced
struct NothingCounted;

#[async_trait]
impl UsageTokenCounter for NothingCounted {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Err(CountError::MissingInput)
    }
}

impl StreamCostCalculator for NothingCounted {
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
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Err(CostError {
            message: "the helper prices nothing".to_owned(),
        })
    }
}

struct Silent;

impl LoggerSink for Silent {
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

impl ConsoleSink for Silent {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

fn decoded<T>(value: &OtelModelValue) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .ok()
}

// the port's chat side is B2227's stream_chunk_builder; the helper streams are typed chunks, so no raw frame arrives
pub(super) struct ChunkBuilderAssembler;

impl StreamOutputAssembler for ChunkBuilderAssembler {
    fn assemble_sse(&self, _frames: &[Vec<u8>]) -> Option<OtelModelValue> {
        None
    }

    fn assemble_chat(
        &self,
        chunks: &[OtelModelValue],
        messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue> {
        let records: Vec<Chunk> = chunks.iter().map(decoded).collect::<Option<_>>()?;
        let typed_messages: Option<Vec<Message>> = match messages {
            Some(items) => Some(items.iter().map(decoded).collect::<Option<_>>()?),
            None => None,
        };
        let hooks = VerboseHooks {
            logger: Silent,
            console: Silent,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).ok()?),
        };
        let request = Request {
            chunks: Some(records.as_slice()),
            messages: typed_messages.as_deref(),
            count_prompt_tokens: None,
            logging: None,
            costs: &NothingCounted,
            counter: &NothingCounted,
            metadata: None,
            now: 0,
        };
        let runtime = Builder::new_current_thread().build().ok()?;
        match runtime
            .block_on(stream_chunk_builder(&request, &hooks))
            .ok()??
        {
            Response::Chat(chat) => serde_json::to_value(&*chat)
                .and_then(serde_json::from_value)
                .ok(),
            Response::Text(_) => None,
        }
    }
}
