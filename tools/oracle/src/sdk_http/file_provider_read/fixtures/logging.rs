use std::time::Duration;

use betterlite_core::api::file_provider_read::target::NewLogger;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset, Utc};
use thiserror::Error;

// the fixture's own failures: a redactor, a logging state or the init record it deserializes
#[derive(Debug, Error)]
#[error("the logging fixture could not be built: {message}")]
pub(crate) struct FixtureError {
    message: String,
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The logging state a production request carries, plus the pre/post environment pre_call records into
pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new() -> Result<Self, FixtureError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| FixtureError {
            message: error.to_string(),
        })?;
        Ok(Self {
            redactor,
            sink: Sink,
            now: Utc::now().fixed_offset(),
        })
    }

    // the object LiteLLM's logging class gets for a call that passed none: model "", the call type and the dispatcher's id and function id
    pub(crate) fn state(&self, new: &NewLogger) -> Result<LoggingState, FixtureError> {
        let call_id = new.litellm_call_id.clone().unwrap_or_default();
        let text = format!(
            r#"{{"model":"","call_type":"{}","start_time":"{}","litellm_call_id":"{call_id}","function_id":"{}","litellm_trace_id":"trace-1"}}"#,
            new.call_type,
            new.start_time.to_rfc3339(),
            new.function_id
        );
        let init: LoggingInit = serde_json::from_str(&text).map_err(|error| FixtureError {
            message: error.to_string(),
        })?;
        let setup = CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        };
        LoggingState::new(
            init,
            setup,
            &self.redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(|error| FixtureError {
            message: error.to_string(),
        })
    }

    pub(crate) fn environment(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}
