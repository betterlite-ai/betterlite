#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;

use axum::Router;
use axum::routing::post;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    GraySwanGuardrail, GraySwanGuardrailAPIError, GraySwanInput, GraySwanPayload, GraySwanResponse,
    GraySwanSettings, OnFlaggedAction,
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

#[test]
fn grayswan_payload_reconstructs_history_and_merges_dynamic_parameters() {
    let mut input = GraySwanInput {
        messages: vec![message("user", "new")],
        tools: vec![GuardrailToolParam {
            tool_type: "function".to_owned(),
            extra: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
        request_history: vec![message("assistant", "old")],
    };
    input.metadata.insert(
        "policy_id".to_owned(),
        GuardrailExtraValue::Text("request-policy".to_owned()),
    );
    let payload = guardrail(OnFlaggedAction::Block).prepare_payload(&input);
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(
        payload.parameters.get("policy_id"),
        Some(&GuardrailExtraValue::Text("request-policy".to_owned()))
    );
}

#[test]
fn grayswan_verdict_at_threshold_blocks() {
    let guardrail = guardrail(OnFlaggedAction::Block);
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: Some("policy".to_owned()),
        messages: None,
    };
    let result = guardrail.process_response(response, GuardrailInputType::Request, vec![]);
    drop(result.expect_err("threshold violation"));
}

#[test]
fn grayswan_passthrough_replaces_post_call_messages() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: None,
        messages: Some(vec![message("assistant", "changed")]),
    };
    let result = guardrail
        .process_response(response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(result.messages, vec![message("assistant", "changed")]);
}

#[test]
fn grayswan_context_reconstructs_request_history() {
    let input = GraySwanInput {
        messages: vec![message("user", "new")],
        tools: Vec::new(),
        metadata: BTreeMap::new(),
        request_history: vec![message("assistant", "old")],
    };
    assert_eq!(
        guardrail(OnFlaggedAction::Monitor)
            .prepare_payload(&input)
            .messages
            .len(),
        2
    );
}

#[test]
fn grayswan_monitor_verdict_passes_through() {
    let response = GraySwanResponse {
        violation_score: 0.5,
        violation_reason: None,
        messages: None,
    };
    let result = guardrail(OnFlaggedAction::Monitor)
        .process_response(response, GuardrailInputType::Request, Vec::new())
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
    let response = GraySwanResponse {
        violation_score: 0.1,
        violation_reason: None,
        messages: None,
    };
    let result = guardrail
        .process_response(response, GuardrailInputType::Request, vec![])
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
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

fn empty_payload() -> GraySwanPayload {
    GraySwanPayload {
        messages: Vec::new(),
        tools: Vec::new(),
        parameters: BTreeMap::new(),
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
    assert!((response.violation_score - 0.0).abs() < f64::EPSILON);
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

fn empty_input() -> GraySwanInput {
    GraySwanInput {
        messages: Vec::new(),
        tools: Vec::new(),
        metadata: BTreeMap::new(),
        request_history: Vec::new(),
    }
}

async fn apply_guardrail_against_a_failing_server(
    fail_open: bool,
) -> Result<super::GraySwanResult, GraySwanGuardrailAPIError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let settings = GraySwanSettings {
        fail_open,
        ..GraySwanSettings::default()
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
        .apply_guardrail(empty_input(), GuardrailInputType::Request)
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
