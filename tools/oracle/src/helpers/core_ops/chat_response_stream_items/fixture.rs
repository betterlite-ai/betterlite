use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;

pub(crate) type Hooks = VerboseHooks<FixtureLogger, FixtureConsole>;

pub(crate) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) fn hooks() -> Result<Hooks, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    Ok(VerboseHooks {
        logger: FixtureLogger,
        console: FixtureConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

// The assembly logs through the hooks; the helper compares only the response, so the sinks record nothing.
#[derive(Default)]
pub(crate) struct FixtureLogger;

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

#[derive(Default)]
pub(crate) struct FixtureConsole;

impl ConsoleSink for FixtureConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

// LiteLLM's logging object prices nothing for these chunks, so the builder sees no cost and counts no tokens.
pub(crate) struct ZeroCounter;

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

pub(crate) struct NoCosts;

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
