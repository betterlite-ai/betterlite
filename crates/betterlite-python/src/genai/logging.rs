use super::convert::{GenaiCall, decision_object};
use super::error::SessionError;
use super::session::Entry;
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
use chrono::Utc;
use indexmap::IndexMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
mod tests;
static NEXT_CALL: AtomicU64 = AtomicU64::new(0);
// LiteLLM's logging object for the call, with the call type its entry point names
pub(crate) fn logging_state(
    call: &GenaiCall,
    entry: Entry,
    redactor: &Redactor,
) -> Result<LoggingState, SessionError> {
    let mut correlation = CorrelationContext::default();
    let init = LoggingInit {
        model: call.model.clone(),
        call_type: entry.call_type().to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: next_call_id(),
        function_id: next_call_id(),
        messages: None,
        stream: Some(matches!(entry, Entry::BlockingStream | Entry::AwaitStream)),
        litellm_trace_id: None,
        kwargs: decision_object(&call.kwargs),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(init, setup, redactor, next_call_id, || Duration::ZERO)
        .map_err(|source| SessionError::Logging { source })
}
// the pre/post env the handler logs through; a Python session has no callbacks, sentry or logger function
pub(crate) fn logging_env<'env>(
    redactor: &'env Redactor,
    sink: &'env SilentSink,
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
        now: Utc::now().fixed_offset(),
    }
}
// LiteLLM mints a fresh call id per request, and nothing in the Python path reads it back
fn next_call_id() -> String {
    let sequence = NEXT_CALL.fetch_add(1, Ordering::Relaxed);
    format!("betterlite-genai-{sequence}")
}
// A Python session has no log sink of its own, so the pre-call records it would write are dropped.
pub(crate) struct SilentSink;
impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}
