use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use super::ops::helper_error;

const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// The sink belongs to no call, so it outlives the environment that borrows it.
static SILENT: Silent = Silent;

pub(super) fn logging_fixture(
    redactor: &Redactor,
) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(helper_error)?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(helper_error)?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &SILENT,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).map_err(helper_error)?,
    };
    Ok((state, env))
}
