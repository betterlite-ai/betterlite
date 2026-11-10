use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup, SecretRedaction,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::Value;

use super::input::ContextSpec;

pub(crate) struct StateSpec<'probe> {
    pub(crate) stream: bool,
    pub(crate) call_type: &'probe str,
    pub(crate) litellm_params: &'probe BTreeMap<String, Value>,
    pub(crate) start_time: DateTime<FixedOffset>,
    pub(crate) trace_id: Option<&'probe str>,
    pub(crate) session_id: Option<&'probe str>,
    pub(crate) dynamic_success: &'probe [String],
    pub(crate) context: &'probe ContextSpec,
    pub(crate) request_correlation: bool,
}

pub(crate) fn build(
    spec: &StateSpec<'_>,
) -> Result<(LoggingState, CorrelationContext), Box<crate::compare::helper::HelperError>> {
    let mut context = CorrelationContext {
        trace_id: spec.context.trace_id.clone(),
        session_id: spec.context.session_id.clone(),
    };
    let settings = CorrelationSettings {
        supports_correlation_logging: true,
        request_correlation_in_logs: spec.request_correlation,
        secret_redaction: SecretRedaction::Enabled,
    };
    let init = init(spec)?;
    let redactor = Redactor::new(16).map_err(|failure| super::value_error(&failure))?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings,
        },
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|failure| super::value_error(&failure))?;
    Ok((state, context))
}

fn init(spec: &StateSpec<'_>) -> Result<LoggingInit, Box<crate::compare::helper::HelperError>> {
    let mut kwargs = IndexMap::new();
    if let Some(session_id) = spec.session_id {
        kwargs.insert(
            "litellm_session_id".to_owned(),
            Json::Text(session_id.to_owned()),
        );
    }
    let dynamic_callbacks = DynamicCallbacks {
        input: None,
        success: (!spec.dynamic_success.is_empty()).then(|| spec.dynamic_success.to_vec()),
        async_success: None,
        failure: None,
        async_failure: None,
    };
    Ok(LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: spec.call_type.to_owned(),
        start_time: spec.start_time,
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(spec.stream),
        litellm_trace_id: spec.trace_id.map(str::to_owned),
        kwargs,
        litellm_params: params(spec.litellm_params)?,
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks,
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    })
}

pub(crate) fn params(
    values: &BTreeMap<String, Value>,
) -> Result<IndexMap<String, Json>, Box<crate::compare::helper::HelperError>> {
    values
        .iter()
        .map(|(name, value)| Ok((name.clone(), decode::<Json>(value)?)))
        .collect()
}

pub(crate) fn decode<T>(value: &Value) -> Result<T, Box<crate::compare::helper::HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(
        serde_json::to_string(value)
            .map_err(|failure| super::value_error(&failure))?
            .as_str(),
    )
    .map_err(|failure| super::value_error(&failure))
}
