use std::future::Future as _;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use betterlite_core::api::chat_response_stream_items::snapshot::StreamAssembly;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::error::SdkHttpError;

// B924 hands its end-of-stream snapshots to B914's builder; the oracle runs the builder with silent seams
pub(crate) struct StreamContext {
    hooks: VerboseHooks<SilentHooks, SilentOutput>,
}

impl StreamContext {
    pub(crate) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        Ok(Self {
            hooks: VerboseHooks {
                logger: SilentHooks,
                console: SilentOutput,
                set_verbose: Payload::Bool(false),
                redactor: Arc::new(redactor),
            },
        })
    }
}

impl StreamAssembly for StreamContext {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        let decoded = chunks
            .iter()
            .map(chunk_record)
            .collect::<Result<Vec<Chunk>, AssemblyError>>()?;
        let costs = NoCost;
        let counter = ZeroTokens;
        let request = Request {
            chunks: Some(&decoded),
            messages: None,
            count_prompt_tokens: None,
            logging: None,
            costs: &costs,
            counter: &counter,
            metadata: None,
            now: 0,
        };
        // The builder's awaited seams are ready on first poll, so polling here needs no second runtime
        let mut builder = std::pin::pin!(stream_chunk_builder(&request, &self.hooks));
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        loop {
            if let Poll::Ready(result) = builder.as_mut().poll(&mut context) {
                return result;
            }
        }
    }
}

fn chunk_record(chunk: &IndexMap<String, Json>) -> Result<Chunk, AssemblyError> {
    let value = serde_json::to_value(chunk)?;
    serde_json::from_value(value).map_err(AssemblyError::from)
}

struct SilentHooks;

impl LoggerSink for SilentHooks {
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

struct SilentOutput;

impl ConsoleSink for SilentOutput {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

// The mock's usage chunk carries the counts, so no tokenizer runs here
struct ZeroTokens;

#[async_trait]
impl UsageTokenCounter for ZeroTokens {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_messages(
        &self,
        _model: &str,
        _messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        Ok(0)
    }
}

// No cost map is loaded for the case, so the builder's cost seam answers nothing
struct NoCost;

impl StreamCostCalculator for NoCost {
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
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}
