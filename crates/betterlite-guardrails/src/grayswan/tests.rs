#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;

use super::wire::{
    GraySwanDynamicParams, GraySwanRuleNumber, GraySwanVerdict, GraySwanViolatedRule,
    GraySwanViolatedRuleFields,
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
        categories: BTreeMap::new(),
        policy_id: String::new(),
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
    let payload = guardrail(OnFlaggedAction::Block).prepare_payload(
        &scan,
        messages.clone(),
        scan.tools.clone(),
    );
    assert_eq!(payload.messages, messages);
    assert_eq!(payload.policy_id.as_deref(), Some("request-policy"));
}
#[test]
fn grayswan_payload_dynamic_categories_override_configured_categories() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.categories =
        BTreeMap::from([("configured".to_owned(), "monitor".to_owned())]);
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.categories = Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]));
    let payload = instance.prepare_payload(&scan, Vec::new(), Vec::new());
    assert_eq!(
        payload.categories,
        Some(BTreeMap::from([("dynamic".to_owned(), "block".to_owned())]))
    );
}
#[test]
fn grayswan_payload_empty_dynamic_categories_fall_back_to_configured_categories() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.categories =
        BTreeMap::from([("configured".to_owned(), "monitor".to_owned())]);
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.categories = Some(BTreeMap::new());
    let payload = instance.prepare_payload(&scan, Vec::new(), Vec::new());
    assert_eq!(
        payload.categories,
        Some(BTreeMap::from([(
            "configured".to_owned(),
            "monitor".to_owned()
        )]))
    );
}
#[test]
fn grayswan_payload_empty_dynamic_policy_id_falls_back_to_configured_policy_id() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.policy_id = "configured-policy".to_owned();
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.policy_id = Some(String::new());
    let payload = instance.prepare_payload(&scan, Vec::new(), Vec::new());
    assert_eq!(payload.policy_id.as_deref(), Some("configured-policy"));
}
#[test]
fn grayswan_payload_falls_back_to_configured_reasoning_mode() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.reasoning_mode = Some(ReasoningMode::Hybrid);
    let scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    let payload = instance.prepare_payload(&scan, Vec::new(), Vec::new());
    assert_eq!(payload.reasoning_mode, Some(ReasoningMode::Hybrid));
}
#[test]
fn grayswan_payload_dynamic_reasoning_mode_overrides_configured_mode() {
    let mut instance = guardrail(OnFlaggedAction::Block);
    instance.settings.reasoning_mode = Some(ReasoningMode::Off);
    let mut scan = input(vec!["new".to_owned()], Vec::new(), Vec::new());
    scan.metadata.reasoning_mode = Some(ReasoningMode::Hybrid);
    let payload = instance.prepare_payload(&scan, Vec::new(), Vec::new());
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
fn grayswan_block_error_preserves_the_detection_fields() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        Some("self_harm"),
        None,
    )];
    scan_response.mutation = Some(true);
    scan_response.ipi = Some(false);
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect_err("blocked");
    let GraySwanGuardrailAPIError::Blocked {
        violation_location,
        violation,
        violated_rules,
        mutation,
        ipi,
        ..
    } = error
    else {
        panic!("expected a Blocked error");
    };
    assert_eq!(violation_location, "output");
    assert!((violation - 0.9).abs() < f64::EPSILON);
    assert_eq!(
        violated_rules,
        vec![GuardrailExtraValue::Object(BTreeMap::from([
            ("rule".to_owned(), GuardrailExtraValue::SignedInteger(6)),
            (
                "name".to_owned(),
                GuardrailExtraValue::Text("self_harm".to_owned())
            ),
        ]))]
    );
    assert_eq!(mutation, Some(true));
    assert_eq!(ipi, Some(false));
}
#[test]
fn grayswan_block_error_marks_a_pre_call_violation_as_the_input_location() {
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.9), GuardrailInputType::Request, vec![])
        .expect_err("blocked");
    let GraySwanGuardrailAPIError::Blocked {
        violation_location, ..
    } = error
    else {
        panic!("expected a Blocked error");
    };
    assert_eq!(violation_location, "input");
}
#[test]
fn grayswan_block_error_status_code_is_bad_request() {
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.9), GuardrailInputType::Request, vec![])
        .expect_err("blocked");
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
#[test]
fn grayswan_passthrough_replaces_post_call_messages() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let result = guardrail
        .process_response(&response(0.5), GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(result.messages.len(), 1);
    assert!(matches!(result.verdict, GraySwanVerdict::Replaced { .. }));
}
#[test]
fn grayswan_passthrough_short_circuits_pre_call_instead_of_replacing_messages() {
    let guardrail = guardrail(OnFlaggedAction::Passthrough);
    let original = vec![message("user", "hello")];
    let result = guardrail
        .process_response(
            &response(0.5),
            GuardrailInputType::Request,
            original.clone(),
        )
        .expect("pre-call passthrough short-circuits instead of erroring");
    assert_eq!(result.messages, original);
    assert!(matches!(
        result.verdict,
        GraySwanVerdict::ShortCircuited { .. }
    ));
}

fn violation_message(verdict: &GraySwanVerdict) -> Option<&str> {
    match verdict {
        GraySwanVerdict::Pass => None,
        GraySwanVerdict::Monitored { violation_message }
        | GraySwanVerdict::Replaced { violation_message }
        | GraySwanVerdict::ShortCircuited { violation_message } => Some(violation_message),
    }
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
        violation_message(&result.verdict),
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
        violation_message(&result.verdict),
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
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): #6 Illegal Activities: content promotes illegal activities."
        )
    );
}
#[test]
fn grayswan_verdict_message_prefixes_an_explicit_zero_rule_number() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(0)),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): #0 Illegal Activities: content promotes illegal activities."
        )
    );
}
#[test]
fn grayswan_verdict_message_prefixes_an_explicit_empty_string_rule_number() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Text(String::new())),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, vec![])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): # Illegal Activities: content promotes illegal activities."
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
        violation_message(&result.verdict),
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
        violation_message(&result.verdict),
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
    assert!(matches!(result.verdict, GraySwanVerdict::Monitored { .. }));
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn grayswan_monitor_verdict_logs_the_flagged_content() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, || {
        guardrail(OnFlaggedAction::Monitor).process_response(
            &response(0.5),
            GuardrailInputType::Request,
            Vec::new(),
        )
    })
    .expect("monitor action");
    assert!(matches!(result.verdict, GraySwanVerdict::Monitored { .. }));
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Monitoring mode - allowing flagged content"));
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
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
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
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
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
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
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
        categories: BTreeMap::new(),
        policy_id: String::new(),
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
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
}

#[tokio::test]
async fn grayswan_apply_guardrail_propagates_the_error_when_fail_open_is_unset() {
    let result = apply_guardrail_against_a_failing_server(false).await;
    assert!(matches!(
        result,
        Err(GraySwanGuardrailAPIError::HttpStatus { .. })
    ));
}

#[derive(Deserialize)]
struct CapturedPayload {
    messages: Vec<GuardrailMessage>,
}

async fn apply_guardrail_capturing_payload(input_type: GuardrailInputType) -> CapturedPayload {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_handler = Arc::clone(&captured);
    let answer = move |body: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        async move {
            *captured_for_body.lock().unwrap() = body.to_vec();
            (StatusCode::OK, "{}".to_owned())
        }
    };
    let router = Router::new().route("/cygnal/monitor", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = GraySwanGuardrail {
        api_key: SecretString::from("secret"),
        api_base: base,
        settings: GraySwanSettings::default(),
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    };
    let scan = input(
        vec!["hello".to_owned()],
        vec![GuardrailExtraValue::Text("call-xyz".to_owned())],
        vec![message("assistant", "previous-context")],
    );
    instance
        .apply_guardrail(scan, input_type)
        .await
        .expect("guardrail call succeeds");
    let body = captured.lock().unwrap().clone();
    serde_json::from_slice(&body).expect("captured payload is valid JSON")
}

#[tokio::test]
async fn grayswan_apply_guardrail_drops_tool_calls_and_history_for_a_request() {
    let payload = apply_guardrail_capturing_payload(GuardrailInputType::Request).await;
    assert_eq!(payload.messages.len(), 1);
    assert_eq!(payload.messages[0].role, "user");
    assert_eq!(
        payload.messages[0].extra.get("content"),
        Some(&GuardrailExtraValue::Text("hello".to_owned()))
    );
    assert!(!payload.messages[0].extra.contains_key("tool_calls"));
}

#[derive(Deserialize)]
struct CapturedToolsPayload {
    #[serde(default)]
    tools: Option<Vec<GuardrailToolParam>>,
}

async fn apply_guardrail_capturing_raw_payload(scan: GraySwanInput) -> CapturedToolsPayload {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_handler = Arc::clone(&captured);
    let answer = move |body: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        async move {
            *captured_for_body.lock().unwrap() = body.to_vec();
            (StatusCode::OK, "{}".to_owned())
        }
    };
    let router = Router::new().route("/cygnal/monitor", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = GraySwanGuardrail {
        api_key: SecretString::from("secret"),
        api_base: base,
        settings: GraySwanSettings::default(),
        client: reqwest::Client::default(),
        timeout: super::DEFAULT_TIMEOUT,
    };
    instance
        .apply_guardrail(scan, GuardrailInputType::Request)
        .await
        .expect("guardrail call succeeds");
    let body = captured.lock().unwrap().clone();
    serde_json::from_slice(&body).expect("captured payload is valid JSON")
}

#[tokio::test]
async fn grayswan_payload_omits_tools_on_a_pre_call_request() {
    let mut scan = single_message_input();
    scan.tools.push(GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    });
    let payload = apply_guardrail_capturing_raw_payload(scan).await;
    assert!(payload.tools.is_none());
}

#[tokio::test]
async fn grayswan_apply_guardrail_sends_tool_calls_and_history_for_a_response() {
    let payload = apply_guardrail_capturing_payload(GuardrailInputType::Response).await;
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(payload.messages[0].role, "assistant");
    assert_eq!(
        payload.messages[0].extra.get("content"),
        Some(&GuardrailExtraValue::Text("previous-context".to_owned()))
    );
    assert_eq!(
        payload.messages[1].extra.get("content"),
        Some(&GuardrailExtraValue::Text("hello".to_owned()))
    );
    assert!(payload.messages[1].extra.contains_key("tool_calls"));
}

#[test]
fn grayswan_response_parses_a_numeric_violation_string() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": "0.9"}"#).unwrap();
    assert!((parsed.violation - 0.9).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_parses_a_plain_numeric_violation() {
    let bytes = br#"{"violation": 0.9}"#;
    let parsed: GraySwanResponse = serde_json::from_slice(bytes).unwrap();
    assert!((parsed.violation - 0.9).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_defaults_a_null_violation_to_zero() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": null}"#).unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_parses_an_empty_violation_string_as_zero() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": ""}"#).unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_defaults_a_missing_violation_to_zero() {
    let parsed: GraySwanResponse = serde_json::from_str("{}").unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_rejects_a_non_numeric_violation_string() {
    let error = serde_json::from_str::<GraySwanResponse>(r#"{"violation": "not-a-number"}"#)
        .expect_err("invalid violation text is rejected");
    assert!(error.to_string().contains("invalid violation score"));
}

#[test]
fn grayswan_response_with_a_numeric_string_violation_is_blocked_instead_of_passed_through() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": "0.9"}"#).unwrap();
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&parsed, GuardrailInputType::Request, Vec::new())
        .expect_err("a normalized violation above the threshold is blocked");
    assert!(matches!(error, GraySwanGuardrailAPIError::Blocked { .. }));
}

#[test]
fn replays_the_grayswan_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/grayswan_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        serde_json::from_slice::<GraySwanResponse>(&bytes).is_ok()
    }));
}
