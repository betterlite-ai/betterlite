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
use chrono::Utc;
use indexmap::IndexMap;
use uuid::Uuid;

use super::EvalCore;

// The pre-call update every eval service runs needs a logging state; the Python package registers no callbacks.
pub(super) fn call_logging(
    core: &EvalCore,
    call_type: &str,
    kwargs: &IndexMap<String, Json>,
) -> Result<LoggingState, LoggingStateError> {
    let litellm_call_id = match kwargs.get("litellm_call_id") {
        Some(Json::Text(id)) => id.clone(),
        _ => Uuid::new_v4().to_string(),
    };
    let init = LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id,
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
    };
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &core.redactor,
        || Uuid::new_v4().to_string(),
        Duration::default,
    )
}

// The Python eval calls register no callbacks, so the records the pre-call writes have nowhere to go.
pub(super) struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}

// one sink serves every call: the environment borrows it for as long as the logging state lives
pub(super) static SILENT_SINK: SilentSink = SilentSink;

// The environment a pre-call reads: no debug output and no callbacks, with the content decision LiteLLM's level leaves on.
pub(super) fn pre_post_env<'env>(core: &'env EvalCore, sink: &'env SilentSink) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: true,
            key_info: true,
        },
        redactor: &core.redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}
