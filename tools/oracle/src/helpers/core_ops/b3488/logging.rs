use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::Input;

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn clock() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()
}

// The environment pre_call reads: no callbacks, no debug output and no redaction of the content the case carries
pub(super) fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
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
        now,
    }
}

// The Logging object the Python helper builds: the call's model, stream flag and generate_content call type
pub(super) fn logging_state(
    input: &Input,
    redactor: &Redactor,
) -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: input.model.clone(),
            call_type: "generate_content".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .or_value_error()?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(input.stream),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_value_error()
}
