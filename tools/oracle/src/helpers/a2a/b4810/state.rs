use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::compare::helper::HelperError;
use crate::helpers::a2a::b4810::helper_error;

pub(super) const FIXED_START: &str = "2026-01-01T00:00:00+00:00";

pub(super) fn fixed_time(text: &str) -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map_err(|error| helper_error("InvalidTime", error.to_string()))
}

pub(super) fn state(details: &IndexMap<String, Json>) -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_error("RedactorError", error.to_string()))?;
    let mut logging = LoggingState::new(
        LoggingInit {
            model: "a2a_agent/agent".to_owned(),
            call_type: "asend_message_streaming".to_owned(),
            start_time: fixed_time(FIXED_START)?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
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
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| helper_error("LoggingStateError", error.to_string()))?;
    logging.model_call_details.clone_from(details);
    Ok(logging)
}

pub(super) fn details_value(logging: &LoggingState) -> Result<serde_json::Value, Box<HelperError>> {
    let mut details = logging.model_call_details.clone();
    details.shift_remove("litellm_trace_id");
    serde_json::to_value(details)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}
