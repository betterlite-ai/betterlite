use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

const INIT: &str = r#"{"model":"speech-model","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The B249 pre-call the speech arms log through, with the state a test can read the record back from
pub struct SpeechLogging {
    pub redactor: Redactor,
    pub sink: Sink,
    pub state: LoggingState,
    pub now: DateTime<FixedOffset>,
}

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("the logging fixture request does not parse")]
    Init(#[from] serde_json::Error),
    #[error(transparent)]
    Redactor(#[from] RedactionError),
    #[error(transparent)]
    State(#[from] LoggingStateError),
    #[error("the logging fixture clock does not parse")]
    Clock(#[from] chrono::ParseError),
}

pub fn speech_logging() -> Result<SpeechLogging, FixtureError> {
    let init: LoggingInit = serde_json::from_str(INIT)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )?;
    Ok(SpeechLogging {
        redactor,
        sink: Sink,
        state,
        now,
    })
}
