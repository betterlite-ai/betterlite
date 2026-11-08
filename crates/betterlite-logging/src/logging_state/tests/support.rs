use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Serialize;
use std::time::Duration;

use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::LoggingStateError;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn moment(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).expect("the fixture is RFC 3339")
}

pub(crate) fn object(pairs: &[(&str, Json)]) -> Json {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    Json::Object(values)
}

pub(crate) fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    values
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).expect("the fixture is finite"))
}

pub(crate) fn encoded<T>(value: &T) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).expect("the fixture is serializable")
}

pub(crate) fn init() -> LoggingInit {
    LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "completion".to_owned(),
        start_time: moment("2026-01-01T00:00:00+00:00"),
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
    }
}

pub(crate) fn build(init: LoggingInit) -> LoggingState {
    let mut context = CorrelationContext::default();
    build_in(init, &mut context, CorrelationSettings::default())
}

pub(crate) fn build_clocked<Clock>(init: LoggingInit, clock: Clock) -> LoggingState
where
    Clock: FnMut() -> Duration,
{
    try_build(init, clock).expect("the fixture builds")
}

pub(crate) fn try_build<Clock>(
    init: LoggingInit,
    clock: Clock,
) -> Result<LoggingState, LoggingStateError>
where
    Clock: FnMut() -> Duration,
{
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        clock,
    )
}

pub(crate) fn build_in(
    init: LoggingInit,
    context: &mut CorrelationContext,
    settings: CorrelationSettings,
) -> LoggingState {
    LoggingState::new(
        init,
        CorrelationSetup { context, settings },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}
