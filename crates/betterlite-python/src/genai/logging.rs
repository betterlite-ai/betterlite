use super::convert::decision_object;
use super::error::SessionError;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::ports::IdSource;
use betterlite_vertex::genai::session::Entry;
use betterlite_vertex::genai::session::call::GenaiCall;
use chrono::Utc;
use indexmap::IndexMap;
use std::time::Duration;
mod tests;
// LiteLLM's logging object for the call, with the call type its entry point names
pub(crate) fn logging_state(
    call: &GenaiCall,
    entry: Entry,
    redactor: &Redactor,
    ids: &dyn IdSource,
) -> Result<LoggingState, SessionError> {
    let mut correlation = CorrelationContext::default();
    let init = LoggingInit {
        model: call.model.clone(),
        call_type: entry.call_type().to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: ids.next_call_id(),
        function_id: ids.next_call_id(),
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
    LoggingState::new(
        init,
        setup,
        redactor,
        || ids.next_call_id(),
        || Duration::ZERO,
    )
    .map_err(|source| SessionError::Logging { source })
}
// A Python session has no log sink of its own, so the pre-call records it would write are dropped.
pub(crate) struct SilentSink;
impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}
