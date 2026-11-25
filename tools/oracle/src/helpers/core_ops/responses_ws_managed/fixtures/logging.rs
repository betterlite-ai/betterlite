use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::{from_value, json};

use super::FixtureError;

// 2026-01-01T00:00:05Z, the instant every dispatch records at
const NOW_SECONDS: i64 = 1_767_225_605;

pub(in crate::helpers::core_ops::responses_ws_managed) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(in crate::helpers::core_ops::responses_ws_managed) struct LoggingProbe {
    pub(in crate::helpers::core_ops::responses_ws_managed) redactor: Redactor,
    pub(in crate::helpers::core_ops::responses_ws_managed) sink: Sink,
    pub(in crate::helpers::core_ops::responses_ws_managed) state: LoggingState,
    pub(in crate::helpers::core_ops::responses_ws_managed) now: DateTime<FixedOffset>,
}

impl LoggingProbe {
    pub(in crate::helpers::core_ops::responses_ws_managed) fn new() -> Result<Self, FixtureError> {
        let Some(instant) = DateTime::<Utc>::from_timestamp(NOW_SECONDS, 0) else {
            return Err(FixtureError::Clock);
        };
        let now = instant.fixed_offset();
        let init: LoggingInit = from_value(json!({
            "model": "gpt-4o-mini",
            "call_type": "responses",
            "start_time": now.to_rfc3339(),
            "litellm_call_id": "call-1",
            "function_id": "fn-1",
            "litellm_trace_id": "trace-1",
        }))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
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
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
        })
    }
}
