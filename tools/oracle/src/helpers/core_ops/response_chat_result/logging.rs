use std::fmt::Display;
use std::time::Duration;

use chrono::DateTime;
use indexmap::IndexMap;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn logging_state(recorded: Option<&str>) -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    let mut state = LoggingState::new(
        LoggingInit {
            model: "caller-model".to_owned(),
            call_type: "completion".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .map_err(failure)?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(false),
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(failure)?;
    if let Some(recorded) = recorded {
        state.model_call_details.insert(
            "original_response".to_owned(),
            Json::Text(recorded.to_owned()),
        );
    }
    Ok(state)
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
