use std::time::Duration;

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

// The pre-call update every service runs needs a logging state; the Python package registers no callbacks.
pub(super) fn state(
    redactor: &Redactor,
    call_type: &str,
    kwargs: &IndexMap<String, Json>,
    litellm_call_id: Option<&str>,
) -> Result<LoggingState, LoggingStateError> {
    let init = LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: litellm_call_id.map_or_else(|| Uuid::new_v4().to_string(), str::to_owned),
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
        redactor,
        || Uuid::new_v4().to_string(),
        Duration::default,
    )
}
