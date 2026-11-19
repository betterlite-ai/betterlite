use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

pub(super) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// the reply transforms log through a state and an environment, and cost is all the cases compare
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Silent,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, Box<HelperError>> {
        Ok(Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?,
            sink: Silent,
            now: DateTime::parse_from_rfc3339(NOW).or_value_error()?,
        })
    }

    pub(super) fn state(&self) -> Result<LoggingState, Box<HelperError>> {
        LoggingState::new(
            serde_json::from_str::<LoggingInit>(INIT).or_value_error()?,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || String::from("trace"),
            Duration::default,
        )
        .or_value_error()
    }

    pub(super) fn env(&self) -> PrePostEnv<'_> {
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
