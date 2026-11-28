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
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::Utc;
use indexmap::IndexMap;
use uuid::Uuid;

use super::EvalCore;

// The logging state every run service updates before its provider call; the Python package registers no callbacks.
pub(super) fn state(
    core: &EvalCore,
    call_type: &str,
    kwargs: &IndexMap<String, Json>,
) -> Result<LoggingState, LoggingStateError> {
    let call_id = match kwargs.get("litellm_call_id") {
        Some(Json::Text(id)) => id.clone(),
        _ => Uuid::new_v4().to_string(),
    };
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        init(call_type, call_id, kwargs),
        CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        &core.redactor,
        || Uuid::new_v4().to_string(),
        Duration::default,
    )
}

// the state's fixed part: the call type, the caller's call id and the keywords the call carries
fn init(call_type: &str, call_id: String, kwargs: &IndexMap<String, Json>) -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: call_id,
        function_id: String::new(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: kwargs.clone(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

// The Python run calls register no callbacks, so the records a pre-call writes have nowhere to go.
pub(super) struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}

// one sink serves every call: the environment borrows it for as long as the logging state lives
pub(super) static SILENT_SINK: SilentSink = SilentSink;

// The environment a pre-call reads: no debug output and no callbacks, with the content decision LiteLLM's level leaves on.
pub(super) fn env<'env>(redactor: &'env Redactor, sink: &'env SilentSink) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: true,
            key_info: true,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}
