use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use super::FixtureError;

struct Discard;
impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(redactor: &Redactor) -> Result<(LoggingState, PrePostEnv<'_>), FixtureError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"claude","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call","function_id":"function","litellm_trace_id":"trace"}"#,
    )?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace".to_owned(),
        Duration::default,
    )?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::from(
            DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default(),
        ),
    };
    Ok((state, env))
}
