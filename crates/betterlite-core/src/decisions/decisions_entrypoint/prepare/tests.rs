#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_models::decisions::{DecisionsJson, Json, ir};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::fields::RequestFormat;
use secrecy::{ExposeSecret, SecretString};

use super::prepare_decisions_call;
use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, API_KEY, Fixture, decisions_args, json, prepare, systemone_questions, text,
};
use crate::decisions::{DecisionsEntryError, FieldShapeError};

#[test]
fn prepare_filters_state_from_the_litellm_params() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("typesafe");
    args.input = Some(text("ignored"));
    args.questions = Some(systemone_questions());
    args.kwargs.insert(String::from("state"), text("state"));
    args.kwargs
        .insert(String::from("drop_params"), Json::Bool(true));
    let call = prepare_decisions_call(prepare(&resolution, &fixture.logging), args)
        .expect("the call prepares");
    assert!(matches!(call.request, RequestFormat::SystemOne(_)));
    assert_eq!(
        call.ir_request.input,
        ir::DecisionInput::State(DecisionsJson::Text(String::from("state")))
    );
    assert_eq!(
        call.litellm_params.get("drop_params"),
        Some(&Json::Bool(true))
    );
    assert!(!call.litellm_params.contains_key("state"));
}

#[test]
fn prepare_keeps_the_openai_request_and_the_connection_values() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("openai");
    args.extra_headers = Some(BTreeMap::from([(
        String::from("x-test"),
        String::from("1"),
    )]));
    let call = prepare_decisions_call(prepare(&resolution, &fixture.logging), args)
        .expect("the call prepares");
    assert!(matches!(call.request, RequestFormat::OpenAi(_)));
    assert_eq!(call.requested_model, "model-1");
    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(call.api_base, API_BASE);
    assert_eq!(call.timeout, Some(Duration::from_secs(5)));
    assert_eq!(call.headers.get("x-test").map(String::as_str), Some("1"));
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some(API_KEY)
    );
}

#[test]
fn prepare_forwards_the_logging_object() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let call = prepare_decisions_call(
        prepare(&resolution, &fixture.logging),
        decisions_args("typesafe"),
    )
    .expect("the call prepares");
    assert!(call.logging_obj.is_some());
    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.model, "model-1");
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&Json::Text(format!("{API_BASE}/v1/systemone")))
    );
}

#[test]
fn prepare_reports_a_wrong_shaped_questions_value() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("typesafe");
    args.questions = Some(json(r#"{"first":{"type":"noul"}}"#));
    let error = prepare_decisions_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("a map is not an OpenAI question list");
    let DecisionsEntryError::Fields { source } = &error else {
        panic!("a wrong questions value is a field failure");
    };
    assert_eq!(source, &FieldShapeError::QuestionsNotAList);
    assert_eq!(error.class().metadata().status_code, 400);
    assert!(error.to_string().contains("questions"));
}

#[test]
fn prepare_uses_the_callers_globals_for_openai_credentials() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("openai");
    args.api_base = None;
    args.api_key = None;
    let mut caller = prepare(&resolution, &fixture.logging);
    caller.globals = LiteLlmGlobals {
        api_base: Some(String::from("http://127.0.0.1:8")),
        api_key: Some(SecretString::from(String::from("global-key"))),
        openai_key: None,
        organization: None,
    };
    let call = prepare_decisions_call(caller, args).expect("the call prepares");
    assert_eq!(call.api_base, "http://127.0.0.1:8");
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("global-key")
    );
}

#[test]
fn prepare_drops_an_unsupported_safety_identifier_through_the_caller_switch() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("typesafe");
    args.safety_identifier = Some(String::from("user-7"));
    let mut caller = prepare(&resolution, &fixture.logging);
    caller.global_drop_params = true;
    let call = prepare_decisions_call(caller, args).expect("the call prepares");
    assert_eq!(call.ir_request.safety_identifier, None);
}

#[test]
fn prepare_rejects_an_unsupported_safety_identifier_without_the_switch() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = decisions_args("typesafe");
    args.safety_identifier = Some(String::from("user-7"));
    let error = prepare_decisions_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("typesafe does not take a safety identifier");
    assert_eq!(
        error.to_string(),
        "typesafe does not support parameters: ['safety_identifier'], for model=model-1. To drop these, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\n"
    );
    assert_eq!(error.class().metadata().status_code, 400);
}
