use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::input::LogFields;

pub(super) fn call_log(fields: &LogFields) -> Result<LoggingState, Box<HelperError>> {
    let start_time = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
        .map_err(|error| failure(error.to_string()))?;
    let redactor = Redactor::new(16).map_err(|error| failure(error.to_string()))?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let mut state = LoggingState::new(
        init(fields, start_time),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| failure(error.to_string()))?;
    for (name, value) in &fields.details {
        state.model_call_details.insert(name.clone(), value.clone());
    }
    Ok(state)
}

fn init(fields: &LogFields, start_time: DateTime<FixedOffset>) -> LoggingInit {
    LoggingInit {
        model: fields.model.clone(),
        call_type: fields.call_type.clone(),
        start_time,
        litellm_call_id: "stream-usage-cost".to_owned(),
        function_id: "stream-usage-cost".to_owned(),
        messages: None,
        stream: Some(true),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
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

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
