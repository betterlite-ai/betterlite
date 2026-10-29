#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::time::Duration;

use axum::Router;
use axum::routing::post;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    GraySwanGuardrail, GraySwanGuardrailAPIError, GraySwanInput, GraySwanPayload, GraySwanResponse,
    GraySwanSettings, OnFlaggedAction, ReasoningMode,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};

fn guardrail(action: OnFlaggedAction) -> GraySwanGuardrail {
    let settings = GraySwanSettings {
        on_flagged_action: action,
        violation_threshold: 0.5,
        reasoning_mode: None,
        categories: None,
        policy_id: None,
        fail_open: true,
    };
    GraySwanGuardrail::with_client(
        SecretString::from("secret"),
        Some("http://example".to_owned()),
        settings,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
    )
}

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailExtraValue::Text(content.to_owned()),
        )]),
    }
}

fn input(messages: Vec<GuardrailMessage>, request_history: Vec<GuardrailMessage>) -> GraySwanInput {
    GraySwanInput {
        messages,
        tools: Vec::new(),
        metadata: BTreeMap::new(),
        request_history,
        litellm_metadata: BTreeMap::new(),
    }
}

fn response(violation: f64) -> GraySwanResponse {
    GraySwanResponse {
        violation,
        violated_rule_descriptions: Vec::new(),
        mutation: None,
        ipi: None,
    }
}

#[test]
fn grayswan_payload_reconstructs_history_and_merges_dynamic_parameters() {
    let mut scan = input(
        vec![message("user", "new")],
        vec![message("assistant", "old")],
    );
    scan.tools.push(GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    });
    scan.metadata.insert(
        "policy_id".to_owned(),
        GuardrailExtraValue::Text("request-policy".to_owned()),
    );
    let payload = guardrail(OnFlaggedAction::Block).prepare_payload(&scan);
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(payload.policy_id.as_deref(), Some("request-policy"));
}

#[test]
fn grayswan_payload_dynamic_categories_override_configured_categories() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.categories = Some(BTreeMap::from([(
        "configured".to_owned(),
        "monitor".to_owned(),
    )]));
    let mut scan = input(vec![message("user", "new")], Vec::new());
    scan.metadata.insert(
        "categories".to_owned(),
        GuardrailExtraValue::Object(BTreeMap::from([(
            "dynamic".to_owned(),
            GuardrailExtraValue::Text("block".to_owned()),
        )])),
    );
    let payload = instance.prepare_payload(&scan);
    assert_eq!(
        payload.categories,
        Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]))
    );
}

#[test]
fn grayswan_payload_falls_back_to_configured_reasoning_mode() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.reasoning_mode = Some(ReasoningMode::Hybrid);
    let scan = input(vec![message("user", "new")], Vec::new());
    let payload = instance.prepare_payload(&scan);
    assert_eq!(payload.reasoning_mode, Some(ReasoningMode::Hybrid));
}

#[test]
fn grayswan_verdict_at_threshold_blocks() {
    let guardrail = guardrail(OnFlaggedAction::Block);
    let result = guardrail.process_response(&response(0.5), GuardrailInputType::Request, vec![]);
    drop(result.expect_err("threshold violation"));
}

#[test]
fn grayswan_passthrough_replaces_post_call_messages() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let result = guardrail
        .process_response(&response(0.5), GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(result.messages.len(), 1);
    assert!(result.violation_message.is_some());
}

#[test]
fn grayswan_passthrough_raises_on_pre_call() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let result = guardrail.process_response(&response(0.5), GuardrailInputType::Request, vec![]);
    assert!(matches!(
        result,
        Err(GraySwanGuardrailAPIError::PreCallPassthrough { .. })
    ));
}

#[test]
fn grayswan_context_reconstructs_request_history() {
    let scan = input(
        vec![message("user", "new")],
        vec![message("assistant", "old")],
    );
    assert_eq!(
        guardrail(OnFlaggedAction::Monitor)
            .prepare_payload(&scan)
            .messages
            .len(),
        2
    );
}

#[test]
fn grayswan_monitor_verdict_passes_through() {
    let result = guardrail(OnFlaggedAction::Monitor)
        .process_response(&response(0.5), GuardrailInputType::Request, Vec::new())
        .expect("monitor action");
    assert!(result.flagged);
}

#[test]
fn grayswan_error_policy_is_fail_open_by_default() {
    assert!(guardrail(OnFlaggedAction::Monitor).settings.fail_open);
}

#[test]
fn grayswan_below_threshold_passes_through_even_when_the_action_is_block() {
    let guardrail = guardrail(OnFlaggedAction::Block);
    let result = guardrail
        .process_response(&response(0.1), GuardrailInputType::Request, vec![])
        .expect("below threshold is not blocked");
    assert!(!result.flagged);
}

#[test]
fn grayswan_headers_include_bearer_token_and_json_content_type() {
    let instance = guardrail(OnFlaggedAction::Block);
    let headers = instance.prepare_headers().expect("headers");
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer secret"
    );
    assert!(headers.get(AUTHORIZATION).unwrap().is_sensitive());
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

fn empty_payload() -> GraySwanPayload {
    GraySwanPayload {
        messages: Vec::new(),
        tools: Vec::new(),
        categories: None,
        policy_id: None,
        reasoning_mode: None,
        metadata: None,
        litellm_metadata: None,
    }
}

async fn call_api_mock(
    status: StatusCode,
    body: String,
) -> Result<GraySwanResponse, GraySwanGuardrailAPIError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let instance = GraySwanGuardrail::with_client(
        SecretString::from("secret"),
        Some(base),
        GraySwanSettings::default(),
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
    );
    let answer = move || async move { (status, body) };
    let router = Router::new().route("/cygnal/monitor", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    instance.call_api(&empty_payload()).await
}

#[tokio::test]
async fn grayswan_call_api_returns_the_response_on_success() {
    let response = call_api_mock(StatusCode::OK, "{}".to_owned())
        .await
        .expect("success");
    assert!((response.violation - 0.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn grayswan_call_api_rejects_a_non_success_status() {
    let result = call_api_mock(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    assert!(matches!(
        result,
        Err(GraySwanGuardrailAPIError::HttpStatus { status })
            if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[test]
fn grayswan_default_settings_match_litellm() {
    let settings = GraySwanSettings::default();
    assert_eq!(settings.on_flagged_action, OnFlaggedAction::Monitor);
    assert!((settings.violation_threshold - 0.5).abs() < f64::EPSILON);
    assert!(settings.fail_open);
    assert_eq!(super::DEFAULT_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn grayswan_apply_guardrail_skips_the_api_call_when_there_are_no_messages() {
    let instance = guardrail(OnFlaggedAction::Block);
    let result = instance
        .apply_guardrail(input(Vec::new(), Vec::new()), GuardrailInputType::Request)
        .await
        .expect("empty scans pass through without calling the API");
    assert!(!result.flagged);
}

fn single_message_input() -> GraySwanInput {
    input(vec![message("user", "hello")], Vec::new())
}

async fn apply_guardrail_against_a_failing_server(
    fail_open: bool,
) -> Result<super::GraySwanResult, GraySwanGuardrailAPIError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let settings = GraySwanSettings {
        on_flagged_action: OnFlaggedAction::Block,
        violation_threshold: 0.5,
        reasoning_mode: None,
        categories: None,
        policy_id: None,
        fail_open,
    };
    let instance = GraySwanGuardrail::with_client(
        SecretString::from("secret"),
        Some(base),
        settings,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
    );
    let answer = || async { (StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()) };
    let router = Router::new().route("/cygnal/monitor", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    instance
        .apply_guardrail(single_message_input(), GuardrailInputType::Request)
        .await
}

#[tokio::test]
async fn grayswan_apply_guardrail_falls_open_on_an_unreachable_api_when_fail_open_is_set() {
    let result = apply_guardrail_against_a_failing_server(true)
        .await
        .expect("fail open passes through");
    assert!(!result.flagged);
}

#[tokio::test]
async fn grayswan_apply_guardrail_propagates_the_error_when_fail_open_is_unset() {
    let result = apply_guardrail_against_a_failing_server(false).await;
    assert!(matches!(
        result,
        Err(GraySwanGuardrailAPIError::HttpStatus { .. })
    ));
}
