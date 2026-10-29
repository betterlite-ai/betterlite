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

use super::wire::{
    GraySwanDynamicParams, GraySwanRuleNumber, GraySwanViolatedRule, GraySwanViolatedRuleFields,
};
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
    GraySwanGuardrail {
        api_key: SecretString::from("secret"),
        api_base: "http://example".to_owned(),
        settings,
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    }
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

fn input(
    texts: Vec<String>,
    tool_calls: Vec<GuardrailExtraValue>,
    request_history: Vec<GuardrailMessage>,
) -> GraySwanInput {
    GraySwanInput {
        texts,
        tool_calls,
        tools: Vec::new(),
        metadata: GraySwanDynamicParams::default(),
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
fn grayswan_payload_forwards_messages_and_merges_dynamic_parameters() {
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.tools.push(GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    });
    scan.metadata.policy_id = Some("request-policy".to_owned());
    let messages = vec![message("user", "new")];
    let payload = guardrail(OnFlaggedAction::Block).prepare_payload(&scan, messages.clone());
    assert_eq!(payload.messages, messages);
    assert_eq!(payload.policy_id.as_deref(), Some("request-policy"));
}

#[test]
fn grayswan_payload_dynamic_categories_override_configured_categories() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.categories = Some(BTreeMap::from([(
        "configured".to_owned(),
        "monitor".to_owned(),
    )]));
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.categories = Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]));
    let payload = instance.prepare_payload(&scan, Vec::new());
    assert_eq!(
        payload.categories,
        Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]))
    );
}

#[test]
fn grayswan_payload_falls_back_to_configured_reasoning_mode() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.reasoning_mode = Some(ReasoningMode::Hybrid);
    let scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    let payload = instance.prepare_payload(&scan, Vec::new());
    assert_eq!(payload.reasoning_mode, Some(ReasoningMode::Hybrid));
}

#[test]
fn grayswan_payload_dynamic_reasoning_mode_overrides_configured_mode() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.reasoning_mode = Some(ReasoningMode::Off);
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.reasoning_mode = Some(ReasoningMode::Hybrid);
    let payload = instance.prepare_payload(&scan, Vec::new());
    assert_eq!(payload.reasoning_mode, Some(ReasoningMode::Hybrid));
}

#[test]
fn grayswan_build_messages_prepends_context_and_uses_request_role() {
    let context = vec![message("assistant", "old")];
    let messages = super::build_messages(
        &["new".to_owned()],
        &[],
        GuardrailInputType::Request,
        context,
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "assistant");
    assert_eq!(messages[1].role, "user");
}

#[test]
fn grayswan_build_messages_merges_single_text_tail_with_tool_calls() {
    let tool_calls = vec![GuardrailExtraValue::Object(BTreeMap::from([(
        "id".to_owned(),
        GuardrailExtraValue::Text("call_1".to_owned()),
    )]))];
    let messages = super::build_messages(
        &["final answer".to_owned()],
        &tool_calls,
        GuardrailInputType::Response,
        Vec::new(),
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "assistant");
    assert!(messages[0].extra.contains_key("tool_calls"));
    assert_eq!(
        messages[0].extra.get("content"),
        Some(&GuardrailExtraValue::Text("final answer".to_owned()))
    );
}

#[test]
fn grayswan_build_messages_appends_a_separate_tool_calls_message_when_multiple_texts_present() {
    let tool_calls = vec![GuardrailExtraValue::Text("call".to_owned())];
    let messages = super::build_messages(
        &["first".to_owned(), "second".to_owned()],
        &tool_calls,
        GuardrailInputType::Response,
        Vec::new(),
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, "assistant");
    assert!(messages[2].extra.contains_key("tool_calls"));
    assert!(!messages[2].extra.contains_key("content"));
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
fn grayswan_passthrough_replaces_pre_call_messages_instead_of_erroring() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let result = guardrail
        .process_response(&response(0.5), GuardrailInputType::Request, vec![])
        .expect("pre-call passthrough no longer errors");
    assert_eq!(result.messages.len(), 1);
    assert!(result.violation_message.is_some());
}

fn structured_rule(
    rule: Option<GraySwanRuleNumber>,
    name: Option<&str>,
    description: Option<&str>,
) -> GraySwanViolatedRule {
    GraySwanViolatedRule::Structured(GraySwanViolatedRuleFields {
        rule,
        name: name.map(ToOwned::to_owned),
        description: description.map(ToOwned::to_owned),
        extra: BTreeMap::new(),
    })
}

#[test]
fn grayswan_verdict_message_includes_rule_name_and_description() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        None,
        Some("self_harm"),
        Some("content promotes self-harm"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        result.violation_message.as_deref(),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): self_harm: content promotes self-harm."
        )
    );
}

#[test]
fn grayswan_verdict_message_includes_rule_name_only_when_description_missing() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(None, Some("self_harm"), None)];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        result.violation_message.as_deref(),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): self_harm."
        )
    );
}

#[test]
fn grayswan_verdict_message_prefixes_the_rule_number_when_present() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        result.violation_message.as_deref(),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): #6 Illegal Activities: content promotes illegal activities."
        )
    );
}

#[test]
fn grayswan_verdict_message_falls_back_to_a_typed_plain_value_for_an_unrecognized_rule_shape() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![GraySwanViolatedRule::Plain(
        GuardrailExtraValue::Text("profanity".to_owned()),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        result.violation_message.as_deref(),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): profanity."
        )
    );
}

#[test]
fn grayswan_verdict_message_falls_back_to_typed_fields_for_a_nameless_rule_object() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        None,
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        result.violation_message.as_deref(),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): {description: content promotes illegal activities, rule: 6}."
        )
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
    let instance = GraySwanGuardrail {
        api_key: SecretString::from("secret"),
        api_base: base,
        settings: GraySwanSettings::default(),
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    };
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

// Hardcoded independently of `MAX_RESPONSE_BYTES`, so a mutation of that constant can't move this boundary too.
const SPEC_MAX_RESPONSE_BYTES: usize = 0x0010_0000;

#[tokio::test]
async fn grayswan_call_api_allows_a_body_at_the_size_limit() {
    let body = format!("{{}}{}", " ".repeat(SPEC_MAX_RESPONSE_BYTES - 2));
    assert_eq!(body.len(), SPEC_MAX_RESPONSE_BYTES);
    call_api_mock(StatusCode::OK, body)
        .await
        .expect("body at the limit is accepted");
}

#[tokio::test]
async fn grayswan_call_api_rejects_a_body_over_the_size_limit() {
    let body = " ".repeat(SPEC_MAX_RESPONSE_BYTES + 1);
    let result = call_api_mock(StatusCode::OK, body).await;
    assert!(matches!(
        result,
        Err(GraySwanGuardrailAPIError::ResponseTooLarge { limit }) if limit == SPEC_MAX_RESPONSE_BYTES
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
        .apply_guardrail(
            input(Vec::new(), Vec::new(), Vec::new()),
            GuardrailInputType::Request,
        )
        .await
        .expect("empty scans pass through without calling the API");
    assert!(!result.flagged);
}

#[tokio::test]
async fn grayswan_apply_guardrail_ignores_tool_calls_on_pre_call_requests() {
    let instance = guardrail(OnFlaggedAction::Block);
    let scan = input(
        Vec::new(),
        vec![GuardrailExtraValue::Text("call".to_owned())],
        Vec::new(),
    );
    let result = instance
        .apply_guardrail(scan, GuardrailInputType::Request)
        .await
        .expect("tool calls are not scanned on a pre-call request");
    assert!(!result.flagged);
}

fn single_message_input() -> GraySwanInput {
    input(vec!["hello".to_owned()], Vec::new(), Vec::new())
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
    let instance = GraySwanGuardrail {
        api_key: SecretString::from("secret"),
        api_base: base,
        settings,
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    };
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
