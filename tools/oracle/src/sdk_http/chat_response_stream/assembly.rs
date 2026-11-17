use std::future::Future;
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

type Hooks = VerboseHooks<FixtureLogger, FixtureConsole>;

// The driver hands its end-of-stream snapshots to B914's builder, which the oracle runs with quiet sinks
pub(super) struct AssemblyContext {
    hooks: Hooks,
}

impl AssemblyContext {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        Ok(Self {
            hooks: VerboseHooks {
                logger: FixtureLogger,
                console: FixtureConsole,
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
        let counter = ZeroCounter;
        let costs = NoCosts;
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
        block_on(stream_chunk_builder(&request, &self.hooks))
    }
}

// The builder only awaits the token counter and the cost calculator, and both of this context's are ready
// on their first poll; polling here keeps the assembly synchronous without nesting a second runtime.
fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

pub(super) struct FixtureLogger;

impl LoggerSink for FixtureLogger {
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

pub(super) struct FixtureConsole;

impl ConsoleSink for FixtureConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

// The wire usage chunk carries the counts, so the builder never falls back to counting any text
pub(super) struct ZeroCounter;

#[async_trait]
impl UsageTokenCounter for ZeroCounter {
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

// The mock's usage chunk reports its own cost, so the builder prices nothing itself
pub(super) struct NoCosts;

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
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}
