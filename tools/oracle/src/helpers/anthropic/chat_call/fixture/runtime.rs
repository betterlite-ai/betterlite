use std::convert::Infallible;
use std::time::Duration;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use uuid::Uuid;

const INIT: &str = r#"{"model":"claude-sonnet-4-6","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const ENV_NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Clone, Copy)]
pub(crate) struct FixtureCounter;

impl ReasoningTokenCounter for FixtureCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(crate) struct FixtureIds;

impl IdSource for FixtureIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

pub(crate) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()
}

pub(crate) fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT).or_value_error()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .or_value_error()
}

pub(crate) fn logging_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Silent,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(ENV_NOW).or_value_error()?,
    })
}
