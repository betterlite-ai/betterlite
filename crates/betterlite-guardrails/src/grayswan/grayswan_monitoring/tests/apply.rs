use std::collections::BTreeMap;

use axum::http::StatusCode as AxumStatus;
use betterlite_config::sections::grayswan::OnFlaggedAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{GuardrailExtraValue, GuardrailInputType};
use crate::grayswan::GraySwanGuardrail;
use crate::grayswan::error::{GraySwanFailure, GraySwanGuardrailAPIError};
use crate::grayswan::grayswan_monitoring::Object;
use crate::grayswan::grayswan_monitoring::RequestContext;
use crate::grayswan::wire::{
    GraySwanDynamicParams, GraySwanInput, GraySwanResult, GraySwanVerdict,
};

use crate::grayswan::tests::{base, configured, field, plain_message, text_field};

use super::{answer, capture_server, captured_payload, records, text};

fn instance(api_base: &str, action: OnFlaggedAction, fail_open: bool) -> GraySwanGuardrail {
    GraySwanGuardrail {
        settings: configured(api_base, action, fail_open),
        base: base(),
        client: reqwest::Client::default(),
    }
}

fn input(texts: Vec<String>, tool_calls: Vec<GuardrailExtraValue>) -> GraySwanInput {
    GraySwanInput {
        texts,
        tool_calls,
        tools: Vec::new(),
        metadata: GraySwanDynamicParams::default(),
        request_history: Vec::new(),
    }
}

async fn apply(
    guardrail: &GraySwanGuardrail,
    input: GraySwanInput,
    input_type: GuardrailInputType,
    data: &mut Object,
) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
    guardrail
        .apply_guardrail(
            input,
            input_type,
            RequestContext {
                request_data: data,
                log_target: None,
            },
        )
        .await
}

fn tools() -> Vec<GuardrailExtraValue> {
    vec![GuardrailExtraValue::Object(BTreeMap::from([(
        "type".to_owned(),
        GuardrailExtraValue::Text("function".to_owned()),
    )]))]
}

#[tokio::test]
async fn apply_guardrail_skips_the_call_when_there_is_nothing_to_scan() {
    let (capture, base_url, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let mut data = Object::new();
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Block, false),
        input(Vec::new(), Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("nothing to scan is a pass");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
    assert!(capture.0.lock().unwrap().method.is_none());
}

#[tokio::test]
async fn apply_guardrail_does_not_scan_tool_calls_on_a_request() {
    let (capture, base_url, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.9}"#)).await;
    let tool_calls = vec![GuardrailExtraValue::Text("call".to_owned())];
    let mut data = Object::new();
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Block, false),
        input(Vec::new(), tool_calls.clone()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("request tool calls are not scanned");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
    assert_eq!(result.tool_calls, tool_calls);
    assert!(capture.0.lock().unwrap().method.is_none());
}

#[tokio::test]
async fn apply_guardrail_blocks_a_high_score_even_with_fail_open() {
    let (_capture, base_url, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.9}"#)).await;
    let mut data = Object::new();
    let error = apply(
        &instance(&base_url, OnFlaggedAction::Block, true),
        input(vec!["hello".to_owned()], Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect_err("a block is not swallowed");
    assert!(matches!(error.cause, Some(GraySwanFailure::Blocked { .. })));
}

#[tokio::test]
async fn apply_guardrail_posts_the_request_messages_and_passes_a_low_score() {
    let (capture, base_url, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.1}"#)).await;
    let mut data = Object::new();
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Block, true),
        input(vec!["hello".to_owned()], Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("a low score passes");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
    assert_eq!(result.texts, vec!["hello".to_owned()]);
    let body = captured_payload(&capture);
    assert_eq!(body.messages.len(), 1);
    assert_eq!(body.messages[0].role, "user");
    assert_eq!(body.messages[0].extra.get("content"), Some(&text("hello")));
}

#[tokio::test]
async fn apply_guardrail_monitors_a_high_score() {
    let (_capture, base_url, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.9}"#)).await;
    let mut data = Object::new();
    let original = vec!["hello".to_owned()];
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Monitor, false),
        input(original.clone(), Vec::new()),
        GuardrailInputType::Response,
        &mut data,
    )
    .await
    .expect("monitor allows flagged content");
    assert!(matches!(result.verdict, GraySwanVerdict::Monitored { .. }));
    assert_eq!(result.texts, original);
}

#[tokio::test]
async fn apply_guardrail_fails_open_and_records_the_failure() {
    let (_capture, base_url, _server) =
        capture_server(answer(AxumStatus::INTERNAL_SERVER_ERROR, "{}")).await;
    let mut data = Object::new();
    let original = vec!["hello".to_owned()];
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Block, true),
        input(original.clone(), Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("fail_open allows the request");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
    assert_eq!(result.texts, original);
    let entry = records(&data).first().expect("a failure record");
    assert_eq!(
        field(entry, "guardrail_status"),
        Some(&text("guardrail_failed_to_respond"))
    );
    let response = text_field(entry, "guardrail_response").expect("the failure message");
    assert!(response.starts_with("the Gray Swan API returned status"));
}

#[tokio::test]
async fn apply_guardrail_fails_closed_when_fail_open_is_false() {
    let (_capture, base_url, _server) =
        capture_server(answer(AxumStatus::INTERNAL_SERVER_ERROR, "{}")).await;
    let mut data = Object::new();
    let error = apply(
        &instance(&base_url, OnFlaggedAction::Block, false),
        input(vec!["hello".to_owned()], Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect_err("fail_open is off");
    assert_eq!(error.status_code, Some(500));
    assert_eq!(records(&data).len(), 1);
}

#[tokio::test]
async fn apply_guardrail_sends_response_context_and_tools() {
    let (capture, base_url, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let mut data = Object::new();
    let mut scan = input(vec!["new".to_owned()], Vec::new());
    scan.tools = tools();
    scan.request_history = vec![plain_message("assistant", "old")];
    apply(
        &instance(&base_url, OnFlaggedAction::Monitor, true),
        scan,
        GuardrailInputType::Response,
        &mut data,
    )
    .await
    .expect("a low score passes");
    let body = captured_payload(&capture);
    assert_eq!(body.messages.len(), 2);
    assert_eq!(body.messages[0].role, "assistant");
    assert_eq!(body.messages[0].extra.get("content"), Some(&text("old")));
    assert_eq!(body.messages[1].extra.get("content"), Some(&text("new")));
    assert_eq!(body.tools, Some(tools()));
}

#[tokio::test]
async fn apply_guardrail_omits_context_and_tools_on_a_request() {
    let (capture, base_url, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let mut data = Object::new();
    let mut scan = input(vec!["new".to_owned()], Vec::new());
    scan.tools = tools();
    scan.request_history = vec![plain_message("assistant", "old")];
    apply(
        &instance(&base_url, OnFlaggedAction::Monitor, true),
        scan,
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("a low score passes");
    let body = captured_payload(&capture);
    assert_eq!(body.messages.len(), 1);
    assert_eq!(body.messages[0].role, "user");
    assert!(body.tools.is_none());
}

#[tokio::test]
async fn apply_guardrail_scans_tool_calls_on_a_response() {
    let (capture, base_url, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let tool_calls = vec![GuardrailExtraValue::Text("call".to_owned())];
    let mut data = Object::new();
    let result = apply(
        &instance(&base_url, OnFlaggedAction::Monitor, true),
        input(Vec::new(), tool_calls.clone()),
        GuardrailInputType::Response,
        &mut data,
    )
    .await
    .expect("tool calls alone are scanned");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
    assert_eq!(result.tool_calls, tool_calls);
    let body = captured_payload(&capture);
    assert_eq!(body.messages.len(), 1);
    assert_eq!(body.messages[0].role, "assistant");
    assert_eq!(
        body.messages[0].extra.get("tool_calls"),
        Some(&GuardrailValue::Array(tool_calls))
    );
}

#[tokio::test]
async fn apply_guardrail_forwards_litellm_metadata() {
    let (capture, base_url, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let mut data = Object::from([(
        "litellm_metadata".to_owned(),
        GuardrailValue::Object(BTreeMap::from([("request-id".to_owned(), text("r1"))])),
    )]);
    apply(
        &instance(&base_url, OnFlaggedAction::Monitor, true),
        input(vec!["hello".to_owned()], Vec::new()),
        GuardrailInputType::Request,
        &mut data,
    )
    .await
    .expect("a low score passes");
    let body = captured_payload(&capture);
    assert_eq!(
        body.litellm_metadata,
        Some(GuardrailValue::Object(BTreeMap::from([(
            "request-id".to_owned(),
            text("r1"),
        )])))
    );
}
