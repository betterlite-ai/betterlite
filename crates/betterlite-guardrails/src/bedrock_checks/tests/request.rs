#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    CHECKS_PATH, GUARDRAIL_NAME, MissingCredentials, RUNTIME_URL, ScriptedTransport, Sent,
    StaticCredentials, as_object, as_text, base, built, checks_named, during_call, field,
    guardrail, guardrail_init, input_scan, messages, model_response, only_record, output_scan,
    records, run_checks,
};
use crate::bedrock_message_selection::wire::Message;
use crate::bedrock_records::Response;
use crate::guardrail_base::failure::GuardrailFailure;

const PASS_BODY: &str =
    r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.2}]}}}"#;
const HIT_BODY: &str =
    r#"{"results":{"contentFilter":{"results":[{"category":"VIOLENCE","severityScore":0.8}]}}}"#;
const UNEXPECTED_SHAPE: &str =
    "Bedrock InvokeGuardrailChecks returned an unexpected response shape";

fn data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::new()
}

fn hello() -> Vec<Message> {
    messages(r#"[{"role":"user","content":"hello"}]"#)
}

fn blocking_guardrail() -> BedrockGuardrail {
    let mut init = guardrail_init(checks_named(&["contentFilter"]));
    init.content_filter_threshold = Some(ConfidenceThreshold(0.5));
    built(init)
}

fn encoded(value: &GuardrailValue) -> String {
    serde_json::to_string(value).expect("a guardrail value as JSON")
}

fn encoded_response(response: &Response) -> String {
    serde_json::to_string(response).expect("the checks response as JSON")
}

fn sent_once(transport: &ScriptedTransport) -> Sent {
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    sent.into_iter().next().expect("the checks request")
}

#[tokio::test]
async fn posts_the_checks_call_to_the_guardrail_checks_path() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    let sent = sent_once(&transport);
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, format!("{RUNTIME_URL}{CHECKS_PATH}"));
    assert_eq!(sent.content_type.as_deref(), Some("application/json"));
    let authorization = sent
        .authorization
        .as_deref()
        .expect("the SigV4 authorization header");
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
}

#[tokio::test]
async fn sends_the_checks_and_retags_every_message_as_user() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let scanned = messages(
        r#"[{"role":"system","content":"rules"},{"role":"user","content":[{"type":"text","text":"hello"},{"type":"text","text":"second"}]}]"#,
    );
    let scan = input_scan(&scanned);
    run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    let sent = sent_once(&transport);
    assert_eq!(
        sent.body,
        r#"{"messages":[{"role":"user","content":[{"text":"rules"}]},{"role":"user","content":[{"text":"hello"},{"text":"second"}]}],"checks":{"contentFilter":{},"promptAttack":{},"sensitiveInformation":{}}}"#
    );
}

#[tokio::test]
async fn output_scan_sends_the_assistant_role_and_the_post_call_mode() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let response = model_response(
        r#"{"choices":[{"message":{"role":"assistant","content":"model says hi"}}]}"#,
    );
    let scan = output_scan(&response);
    run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    let sent = sent_once(&transport);
    assert_eq!(
        sent.body,
        r#"{"messages":[{"role":"assistant","content":[{"text":"model says hi"}]}],"checks":{"contentFilter":{},"promptAttack":{},"sensitiveInformation":{}}}"#
    );
    let record = only_record(&request_data);
    assert_eq!(as_text(&field(&record, "guardrail_mode")), "post_call");
}

#[tokio::test]
async fn a_passing_check_writes_the_success_record() {
    let transport = ScriptedTransport::replying(200, PASS_BODY);
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    assert_eq!(encoded_response(&outcome), "{}");
    let record = only_record(&request_data);
    assert_eq!(as_text(&field(&record, "guardrail_name")), GUARDRAIL_NAME);
    assert_eq!(as_text(&field(&record, "guardrail_provider")), "bedrock");
    assert_eq!(as_text(&field(&record, "guardrail_mode")), "pre_call");
    assert_eq!(as_text(&field(&record, "guardrail_status")), "success");
    assert_eq!(encoded(&field(&record, "start_time")), "1764000000.0");
    assert_eq!(encoded(&field(&record, "duration")), "0.0");
    assert_eq!(encoded(&field(&record, "guardrail_response")), PASS_BODY);
    assert!(!record.contains_key("violation_categories"));
    assert!(!record.contains_key("guardrail_action"));
}

#[tokio::test]
async fn a_blocked_check_writes_the_intervened_record() {
    let transport = ScriptedTransport::replying(200, HIT_BODY);
    let mut request_data = data();
    let scanned = messages(r#"[{"role":"user","content":"hate"}]"#);
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &blocking_guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("a blocked check");
    let GuardrailFailure::HttpObject { status, .. } = failure else {
        panic!("expected a 400 http object: {failure:?}");
    };
    assert_eq!(status, 400);
    let record = only_record(&request_data);
    assert_eq!(
        as_text(&field(&record, "guardrail_status")),
        "guardrail_intervened"
    );
    assert_eq!(
        encoded(&field(&record, "violation_categories")),
        r#"["VIOLENCE"]"#
    );
    assert_eq!(
        as_text(&field(&record, "guardrail_action")),
        "GUARDRAIL_INTERVENED"
    );
    assert_eq!(encoded(&field(&record, "guardrail_response")), HIT_BODY);
}

#[tokio::test]
async fn during_call_overrides_the_record_mode() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let scanned = hello();
    let scan = during_call(input_scan(&scanned));
    run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    let record = only_record(&request_data);
    assert_eq!(as_text(&field(&record, "guardrail_mode")), "during_call");
}

#[tokio::test]
async fn nothing_to_scan_returns_before_the_call() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let scanned: Vec<Message> = Vec::new();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("an empty scan");
    assert_eq!(encoded_response(&outcome), "{}");
    assert!(transport.sent().is_empty());
    assert!(records(&request_data).is_empty());
}

#[tokio::test]
async fn the_logged_response_strips_the_pii_offsets() {
    let mut init = guardrail_init(checks_named(&["sensitiveInformation"]));
    init.pii_confidence_threshold = Some(ConfidenceThreshold(0.9));
    let transport = ScriptedTransport::replying(
        200,
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.6,"messageIndex":0,"contentIndex":0,"beginOffset":12,"endOffset":17}]}}}"#,
    );
    let mut request_data = data();
    let scanned = messages(r#"[{"role":"user","content":"my email"}]"#);
    let scan = input_scan(&scanned);
    run_checks(
        &built(init),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await
    .expect("a passing check");
    let record = only_record(&request_data);
    assert_eq!(
        encoded(&field(&record, "guardrail_response")),
        r#"{"results":{"sensitiveInformation":{"results":[{"type":"EMAIL","confidenceScore":0.6}]}}}"#
    );
}

#[tokio::test]
async fn a_non_200_response_is_the_upstream_error() {
    let transport = ScriptedTransport::replying(400, r#"{"message":"bad input"}"#);
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("an upstream error");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("expected a 400 http error: {failure:?}");
    };
    assert_eq!(status, 400);
    assert_eq!(detail, "bad input");
    let record = only_record(&request_data);
    assert_eq!(
        as_text(&field(&record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
    assert_eq!(
        encoded(&field(&record, "guardrail_response")),
        r#"{"error":"bad input"}"#
    );
}

#[tokio::test]
async fn a_success_status_other_than_200_is_an_error() {
    let transport = ScriptedTransport::replying(201, "{}");
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("an unexpected status");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("expected a 201 http error: {failure:?}");
    };
    assert_eq!(status, 201);
    assert_eq!(detail, "Bedrock guardrail request failed");
}

#[tokio::test]
async fn a_malformed_response_is_a_500() {
    let transport = ScriptedTransport::replying(200, r#"{"results":5}"#);
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("a malformed response");
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("expected a 500 http object: {failure:?}");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(UNEXPECTED_SHAPE.to_owned()))
    );
    let record = only_record(&request_data);
    assert_eq!(
        as_text(&field(&record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
    assert!(as_object(&field(&record, "guardrail_response")).contains_key("error"));
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_500() {
    let transport = ScriptedTransport::unreachable();
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("an unreachable endpoint");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("expected a 500 http error: {failure:?}");
    };
    assert_eq!(status, 500);
    assert_eq!(detail, "the Bedrock guardrail request could not be sent");
    let record = only_record(&request_data);
    assert_eq!(
        as_text(&field(&record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
}

#[tokio::test]
async fn an_api_key_of_the_wrong_type_is_a_500_without_a_record() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data =
        BTreeMap::from([("api_key".to_owned(), GuardrailValue::SignedInteger(5))]);
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &StaticCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("a bad api_key");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("expected a 500 http error: {failure:?}");
    };
    assert_eq!(status, 500);
    assert_eq!(detail, "the Bedrock request `api_key` must be a string");
    assert!(transport.sent().is_empty());
    assert!(records(&request_data).is_empty());
}

#[tokio::test]
async fn missing_credentials_are_a_500_without_a_record() {
    let transport = ScriptedTransport::replying(200, "{}");
    let mut request_data = data();
    let scanned = hello();
    let scan = input_scan(&scanned);
    let outcome = run_checks(
        &guardrail(),
        &base(),
        &mut request_data,
        scan,
        &MissingCredentials,
        &transport,
    )
    .await;
    let failure = outcome.expect_err("missing credentials");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("expected a 500 http error: {failure:?}");
    };
    assert_eq!(status, 500);
    assert_eq!(detail, "the Bedrock guardrail credentials cannot be loaded");
    assert!(transport.sent().is_empty());
    assert!(records(&request_data).is_empty());
}
