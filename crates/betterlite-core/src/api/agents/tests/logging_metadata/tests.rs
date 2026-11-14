#![cfg(test)]

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

use crate::api::agents::{AgentLoggingCall, make_logging_obj};

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "call".to_owned(),
            start_time: moment(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn map(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn make_logging_obj_records_model_provider_call_type_and_optional_params() {
    let mut state = logging();
    let kwargs = map(&[]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "waverunner",
            custom_llm_provider: "gemini",
            call_type: "create_agent",
            optional_params: map(&[("name", text("waverunner"))]),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(state.model, "waverunner");
    assert_eq!(state.call_type, "create_agent");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(state.optional_params.get("name"), Some(&text("waverunner")));
    assert_eq!(
        state.model_call_details.get("name"),
        Some(&text("waverunner"))
    );
    assert_eq!(
        state.model_call_details.get("model"),
        Some(&text("waverunner"))
    );
    assert_eq!(
        state.model_call_details.get("call_type"),
        Some(&text("create_agent"))
    );
    assert_eq!(
        state.model_call_details.get("custom_llm_provider"),
        Some(&text("gemini"))
    );
}

#[test]
fn make_logging_obj_applies_an_empty_model() {
    let mut state = logging();
    let kwargs = map(&[]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "",
            custom_llm_provider: "gemini",
            call_type: "list_agents",
            optional_params: IndexMap::new(),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(state.model, "");
    assert!(state.optional_params.is_empty());
}

#[test]
fn make_logging_obj_replaces_the_optional_params() {
    let mut state = logging();
    state
        .optional_params
        .insert("stale".to_owned(), text("value"));
    let kwargs = map(&[]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "waverunner",
            custom_llm_provider: "gemini",
            call_type: "get_agent",
            optional_params: map(&[("name", text("waverunner"))]),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(state.optional_params.len(), 1);
    assert_eq!(state.optional_params.get("name"), Some(&text("waverunner")));
}

#[test]
fn make_logging_obj_carries_the_call_id_from_kwargs() {
    let mut state = logging();
    let kwargs = map(&[("litellm_call_id", text("call-9"))]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "waverunner",
            custom_llm_provider: "gemini",
            call_type: "create_agent",
            optional_params: IndexMap::new(),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&text("call-9"))
    );
}

#[test]
fn make_logging_obj_writes_a_null_call_id_when_kwargs_lack_one() {
    let mut state = logging();
    let kwargs = map(&[]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "waverunner",
            custom_llm_provider: "gemini",
            call_type: "create_agent",
            optional_params: IndexMap::new(),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}

#[test]
fn make_logging_obj_forwards_the_kwargs_metadata() {
    let mut state = logging();
    let metadata = map(&[("user_api_key_alias", text("alias"))]);
    let kwargs = map(&[("metadata", Json::Object(metadata.clone()))]);
    make_logging_obj(
        &mut state,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "waverunner",
            custom_llm_provider: "gemini",
            call_type: "create_agent",
            optional_params: IndexMap::new(),
        },
    )
    .expect("the logging state accepts the update");
    assert_eq!(
        state.litellm_params.get("metadata"),
        Some(&Json::Object(metadata))
    );
}
