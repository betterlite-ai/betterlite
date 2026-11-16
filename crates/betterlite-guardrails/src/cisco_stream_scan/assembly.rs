use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::error::RedactionError;
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
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

use crate::cisco_conversation::Fields;

#[derive(Debug, Error)]
pub(super) enum AssembleError {
    #[error("the guardrail redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}

struct SilentLogs;

impl LoggerSink for SilentLogs {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        tracing::error!("{payload}");
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
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
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

struct ZeroCounter;

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

pub(super) async fn assemble(chunks: &[Chunk]) -> Result<Option<Response>, AssembleError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| AssembleError::Redactor { source })?;
    let hooks = VerboseHooks {
        logger: SilentLogs,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    };
    let costs = NoCosts;
    let counter = ZeroCounter;
    let request = Request {
        chunks: Some(chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: &costs,
        counter: &counter,
        metadata: None,
        now: 0,
    };
    stream_chunk_builder(&request, &hooks)
        .await
        .map_err(AssembleError::from)
}

pub(super) fn response_fields<Value>(response: &Value) -> Result<Fields, serde_json::Error>
where
    Value: Serialize + ?Sized,
{
    let text = serde_json::to_string(response)?;
    let fields = serde_json::from_str(&text)?;
    Ok(fields)
}
