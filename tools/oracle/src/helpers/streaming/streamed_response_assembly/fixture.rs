use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator, StreamLogging,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CounterFixture {
    pub(crate) text: i64,
    pub(crate) messages: i64,
}

pub(crate) struct FixtureCounter {
    fixture: CounterFixture,
}

impl FixtureCounter {
    pub(crate) fn new(fixture: &CounterFixture) -> Self {
        Self {
            fixture: fixture.clone(),
        }
    }
}

#[async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(self.fixture.text)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(self.fixture.text)
    }

    async fn count_messages(
        &self,
        _model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        if messages.is_none() {
            return Err(CountError::MissingInput);
        }
        Ok(self.fixture.messages)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CostFixture {
    pub(crate) completion_cost: Option<Decimal>,
}

pub(crate) struct FixtureCosts {
    fixture: CostFixture,
}

impl FixtureCosts {
    pub(crate) fn new(fixture: &CostFixture) -> Self {
        Self {
            fixture: fixture.clone(),
        }
    }
}

impl StreamCostCalculator for FixtureCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(self.fixture.completion_cost)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Err(CostError {
            message: "fixture cost_per_token is not priced".to_owned(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LoggingFixture {
    pub(crate) provider: Option<String>,
    pub(crate) cache_hit: bool,
    pub(crate) response_cost: Option<Decimal>,
}

pub(crate) struct FixtureLogging {
    fixture: LoggingFixture,
}

impl FixtureLogging {
    pub(crate) fn new(fixture: &LoggingFixture) -> Self {
        Self {
            fixture: fixture.clone(),
        }
    }
}

impl StreamLogging for FixtureLogging {
    fn provider(&self) -> Option<&str> {
        self.fixture.provider.as_deref()
    }

    fn response_cost(&self, _input: &CostInput<'_>) -> Result<Option<Decimal>, CostError> {
        // Logging._response_cost_calculator returns 0.0 before pricing a cache hit.
        if self.fixture.cache_hit {
            return Ok(Some(Decimal::ZERO));
        }
        Ok(self.fixture.response_cost)
    }
}

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

pub(crate) fn hooks() -> Result<VerboseHooks<FixtureLogger, FixtureConsole>, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    Ok(VerboseHooks {
        logger: FixtureLogger,
        console: FixtureConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}

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
