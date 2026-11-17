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

// The driver hands its end-of-stream snapshots to B914's builder, which the oracle runs with quiet sinks
pub(super) struct AssemblyContext {
    hooks: VerboseHooks<Quiet, Quiet>,
}

impl AssemblyContext {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        Ok(Self {
            hooks: VerboseHooks {
                logger: Quiet,
                console: Quiet,
                set_verbose: Payload::Bool(false),
                redactor: Arc::new(redactor),
            },
        })
    }
}

impl StreamAssembly for AssemblyContext {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        let decoded = chunks
            .iter()
            .map(|chunk| {
                let value = serde_json::to_value(chunk)?;
                serde_json::from_value(value).map_err(AssemblyError::from)
            })
            .collect::<Result<Vec<Chunk>, AssemblyError>>()?;
        let request = Request {
            chunks: Some(&decoded),
            messages: None,
            count_prompt_tokens: None,
            logging: None,
            costs: &Quiet,
            counter: &Quiet,
            metadata: None,
            now: 0,
        };
        // The builder's awaited futures are ready on first poll, so polling here needs no second runtime
        let mut future = std::pin::pin!(stream_chunk_builder(&request, &self.hooks));
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }
}

// Every seam the builder calls is silent and counts nothing: the mock's usage chunk carries the counts and cost
pub(super) struct Quiet;

impl LoggerSink for Quiet {
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

impl ConsoleSink for Quiet {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

#[async_trait]
impl UsageTokenCounter for Quiet {
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

impl StreamCostCalculator for Quiet {
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
