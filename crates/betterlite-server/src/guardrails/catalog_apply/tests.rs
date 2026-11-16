#![cfg(test)]

mod errors;
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use axum::http::StatusCode;
use axum::response::Response;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_records::record::GuardrailStatus;
use rstest::rstest;
use serde::Deserialize;

use super::blocked_words::validate_blocked_words;
use super::input_type::resolve;
use super::ports::Body;
use support::{
    FailureHook, LogMode, RunnerMode, SuccessHook, World, async_failure, async_success, body_bytes,
    body_value, kinds, marked_messages, post_call_failure, post_call_success, pre_call, request,
    runner_call, sink_record, submit_failure, submit_success,
};

fn value(text: &str) -> GuardrailValue {
    serde_json::from_str(text).unwrap()
}

fn text(content: &str) -> GuardrailValue {
    GuardrailValue::Text(content.to_owned())
}

const SUCCESS_LOG_KINDS: [&str; 7] = [
    "pre_call",
    "mark_pass_through",
    "runner_call",
    "sink_record",
    "post_call_success",
    "async_success",
    "submit_success",
];
const HTTP_FAILURE_LOG_KINDS: [&str; 5] = [
    "pre_call",
    "mark_pass_through",
    "runner_call",
    "sink_record",
    "post_call_failure",
];
const OTHER_FAILURE_LOG_KINDS: [&str; 7] = [
    "pre_call",
    "mark_pass_through",
    "runner_call",
    "sink_record",
    "async_failure",
    "submit_failure",
    "post_call_failure",
];
const PATCH_FAILURE_KINDS: [&str; 5] = [
    "pre_call",
    "mark_pass_through",
    "async_failure",
    "submit_failure",
    "post_call_failure",
];
const UNLOGGED_SUCCESS_KINDS: [&str; 3] = ["pre_call", "runner_call", "sink_record"];
const UNLOGGED_FAILURE_KINDS: [&str; 4] = [
    "pre_call",
    "runner_call",
    "sink_record",
    "post_call_failure",
];
const UNKNOWN_GUARDRAIL_KINDS: [&str; 1] = ["post_call_failure"];

fn body_map(text: &str) -> Body {
    serde_json::from_str(text).unwrap()
}

async fn apply_request(world: &World, body: &str) -> Response {
    world
        .send(request("POST", "/guardrails/apply_guardrail", body, true))
        .await
}

async fn assert_unauthenticated(world: &World, method: &str, uri: &str) {
    let response = world.send(request(method, uri, "", false)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
        )
    );
}

#[derive(Debug, Deserialize, PartialEq)]
struct CategoryBody {
    category_name: String,
    yaml_content: String,
    file_type: String,
}

async fn category(world: &World, name: &str) -> Response {
    let uri = format!("/guardrails/ui/category_yaml/{name}");
    world.send(request("GET", &uri, "", true)).await
}

#[tokio::test]
async fn category_yaml_returns_the_raw_file_text_and_its_type() {
    let world = World::new();
    world.write_category("bias_gender", "yaml", "blocked_words: []\n");
    let response = category(&world, "bias_gender").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: CategoryBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(
        body,
        CategoryBody {
            category_name: "bias_gender".to_owned(),
            yaml_content: "blocked_words: []\n".to_owned(),
            file_type: "yaml".to_owned(),
        }
    );
}

#[tokio::test]
async fn category_yaml_reports_a_json_file_with_the_yaml_content_key() {
    let world = World::new();
    world.write_category("harm_toxic_abuse_au", "json", "{\"blocked_words\": []}");
    let response = category(&world, "harm_toxic_abuse_au").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: CategoryBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(body.category_name, "harm_toxic_abuse_au");
    assert_eq!(body.yaml_content, "{\"blocked_words\": []}");
    assert_eq!(body.file_type, "json");
}

#[tokio::test]
async fn category_yaml_prefers_yaml_over_json_in_one_folder() {
    let world = World::new();
    world.write_category("bias_gender", "yaml", "from yaml\n");
    world.write_category("bias_gender", "json", "from json\n");
    let response = category(&world, "bias_gender").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: CategoryBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(body.yaml_content, "from yaml\n");
    assert_eq!(body.file_type, "yaml");
}

#[tokio::test]
async fn category_yaml_prefers_the_first_root_over_the_legacy_one() {
    let mut world = World::new();
    world.write_category("bias_gender", "yaml", "bundled\n");
    let legacy = world.outside_path("legacy");
    fs::create_dir_all(legacy.join("categories")).unwrap();
    fs::write(
        legacy.join("categories").join("bias_gender.yaml"),
        "legacy\n",
    )
    .unwrap();
    let mut roots = world.roots.to_vec();
    roots.push(legacy);
    world.roots = Arc::from(roots);
    let response = category(&world, "bias_gender").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: CategoryBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(body.yaml_content, "bundled\n");
}

#[tokio::test]
async fn category_yaml_rejects_a_nul_byte_in_the_name() {
    let world = World::new();
    let response = category(&world, "bad%00name").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":"Invalid category name"}"#)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn category_yaml_rejects_a_symlink_out_of_the_folder() {
    use std::os::unix::fs::symlink;
    let world = World::new();
    fs::write(world.outside_path("real.yaml"), "outside\n").unwrap();
    symlink(
        world.outside_path("real.yaml"),
        world.category_path("escaped", "yaml"),
    )
    .unwrap();
    let response = category(&world, "escaped").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":"Invalid category name"}"#)
    );
}

#[tokio::test]
async fn category_yaml_answers_404_for_an_unknown_name() {
    let world = World::new();
    let response = category(&world, "missing").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":"Category file not found: missing (tried .yaml and .json)"}"#)
    );
}

#[tokio::test]
async fn category_yaml_answers_500_when_the_file_is_not_utf8() {
    let world = World::new();
    world.write_category_bytes("bias_gender", "yaml", &[0xff, 0xfe, 0x00]);
    let response = category(&world, "bias_gender").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":"Error reading category file: stream did not contain valid UTF-8"}"#)
    );
}

#[tokio::test]
async fn category_yaml_needs_a_key() {
    let world = World::new();
    assert_unauthenticated(&world, "GET", "/guardrails/ui/category_yaml/bias_gender").await;
}

#[derive(Debug, Deserialize, PartialEq)]
struct AirlinesBody {
    airlines: Vec<Airline>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Airline {
    id: String,
    #[serde(rename = "match")]
    matches: String,
    tags: Vec<String>,
}

fn airline(text: &str) -> Airline {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn airlines_returns_the_bundled_list_in_file_order() {
    let world = World::new();
    let response = world
        .send(request("GET", "/guardrails/ui/major_airlines", "", true))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: AirlinesBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
    assert_eq!(body.airlines.len(), 351);
    assert_eq!(
        body.airlines.first(),
        Some(&airline(
            r#"{"id":"abx-air","match":"abx air|gb","tags":["airline"]}"#
        ))
    );
    assert_eq!(
        body.airlines.get(2),
        Some(&airline(
            r#"{"id":"aer-lingus","match":"aer lingus|ei","tags":["airline"]}"#
        ))
    );
    assert_eq!(
        body.airlines.last(),
        Some(&airline(
            r#"{"id":"yto-cargo-airlines","match":"yto cargo airlines|yg","tags":["airline"]}"#
        ))
    );
}

#[tokio::test]
async fn airlines_needs_a_key() {
    let world = World::new();
    assert_unauthenticated(&world, "GET", "/guardrails/ui/major_airlines").await;
}

const VALIDATE_URI: &str = "/guardrails/validate_blocked_words_file";

async fn blocked_words_response(world: &World, body: &str) -> Response {
    world.send(request("POST", VALIDATE_URI, body, true)).await
}

async fn blocked_words(world: &World, body: &str) -> (StatusCode, GuardrailValue) {
    let response = blocked_words_response(world, body).await;
    let status = response.status();
    (status, body_value(response).await)
}

fn file_body(content: &str) -> String {
    let fields = BTreeMap::from([("file_content".to_owned(), content.to_owned())]);
    serde_json::to_string(&fields).unwrap()
}

#[rstest]
#[case::missing("{}")]
#[case::empty(r#"{"file_content":""}"#)]
#[tokio::test]
async fn blocked_words_reports_a_missing_or_empty_file_content(#[case] body: &str) {
    let world = World::new();
    let expected = value(r#"{"valid":false,"error":"No file content provided"}"#);
    let (status, verdict) = blocked_words(&world, body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verdict, expected, "{body}");
}

#[tokio::test]
async fn blocked_words_reports_a_yaml_syntax_error() {
    let world = World::new();
    let (status, verdict) = blocked_words(&world, &file_body("blocked_words: [\n")).await;
    assert_eq!(status, StatusCode::OK);
    let GuardrailValue::Object(fields) = verdict else {
        panic!("the verdict is an object");
    };
    assert_eq!(fields.get("valid"), Some(&GuardrailValue::Boolean(false)));
    let Some(GuardrailValue::Text(error)) = fields.get("error") else {
        panic!("a syntax verdict carries an error text");
    };
    assert!(
        error.starts_with("Invalid YAML syntax: "),
        "unexpected text: {error}"
    );
}

#[rstest]
#[case::scalar("just a string\n")]
#[case::other_key("other_words: []\n")]
#[tokio::test]
async fn blocked_words_reports_a_document_without_the_blocked_words_key(#[case] content: &str) {
    let world = World::new();
    let expected = value(
        r#"{"valid":false,"error":"Invalid format: file must contain 'blocked_words' key with a list"}"#,
    );
    let (status, verdict) = blocked_words(&world, &file_body(content)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verdict, expected, "{content}");
}

#[rstest]
#[case::null("blocked_words: null\n")]
#[case::scalar("blocked_words: a word\n")]
#[tokio::test]
async fn blocked_words_reports_a_non_list_blocked_words_value(#[case] content: &str) {
    let world = World::new();
    let expected = value(r#"{"valid":false,"error":"'blocked_words' must be a list"}"#);
    let (status, verdict) = blocked_words(&world, &file_body(content)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verdict, expected, "{content}");
}

#[tokio::test]
async fn blocked_words_collects_entry_messages_in_index_order() {
    let world = World::new();
    let content = "blocked_words:\n  - keyword: ok\n    action: BLOCK\n  - action: MASK\n  - keyword: 5\n    action: block\n    description: 7\n  - just text\n  - []\n";
    let (status, verdict) = blocked_words(&world, &file_body(content)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        verdict,
        value(
            r#"{"valid":false,"errors":["Entry 1: missing 'keyword' field","Entry 2: 'keyword' must be a string","Entry 2: action must be 'BLOCK' or 'MASK', got 'block'","Entry 2: 'description' must be a string","Entry 3: must be an object","Entry 4: must be an object"]}"#
        )
    );
}

#[tokio::test]
async fn blocked_words_accepts_an_empty_list() {
    let world = World::new();
    let (status, verdict) = blocked_words(&world, &file_body("blocked_words: []\n")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        verdict,
        value(r#"{"valid":true,"message":"Valid YAML file with 0 blocked word(s)"}"#)
    );
}

#[tokio::test]
async fn blocked_words_counts_the_valid_entries() {
    let world = World::new();
    let content = "blocked_words:\n  - keyword: one\n    action: BLOCK\n  - keyword: two\n    action: MASK\n    description: a mask\n";
    let (status, verdict) = blocked_words(&world, &file_body(content)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        verdict,
        value(r#"{"valid":true,"message":"Valid YAML file with 2 blocked word(s)"}"#)
    );
}

#[tokio::test]
async fn blocked_words_reads_yaml_1_2_scalars_as_text() {
    let world = World::new();
    let content = "blocked_words:\n  - keyword: yes\n    action: MASK\n";
    let (status, verdict) = blocked_words(&world, &file_body(content)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        verdict,
        value(r#"{"valid":true,"message":"Valid YAML file with 1 blocked word(s)"}"#)
    );
}

#[derive(Deserialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Deserialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    kind: String,
    loc: Vec<GuardrailValue>,
    msg: String,
}

async fn issue_of(world: &World, body: &str) -> ValidationIssue {
    let response = blocked_words_response(world, body).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let raw = body_bytes(response).await;
    let mut parsed: ValidationBody = serde_json::from_slice(&raw).unwrap();
    assert_eq!(parsed.detail.len(), 1, "{body}");
    parsed.detail.remove(0)
}

#[tokio::test]
async fn blocked_words_rejects_a_non_object_json_body() {
    let world = World::new();
    let issue = issue_of(&world, "[]").await;
    assert_eq!(issue.kind, "value_error");
    assert_eq!(issue.loc, [text("body")]);
    assert!(
        issue.msg.starts_with("invalid type: sequence"),
        "unexpected text: {}",
        issue.msg
    );
}

#[tokio::test]
async fn blocked_words_rejects_a_malformed_json_body() {
    let world = World::new();
    let issue = issue_of(&world, "{").await;
    assert_eq!(issue.kind, "json_invalid");
    assert_eq!(issue.loc, [text("body")]);
    assert_eq!(issue.msg, "JSON decode error");
}

#[tokio::test]
async fn blocked_words_rejects_a_non_text_value() {
    let world = World::new();
    let issue = issue_of(&world, r#"{"file_content": 5}"#).await;
    assert_eq!(issue.kind, "value_error");
    assert_eq!(issue.loc, [text("body"), text("file_content")]);
    assert!(
        issue.msg.starts_with("invalid type: integer"),
        "unexpected text: {}",
        issue.msg
    );
}

#[tokio::test]
async fn blocked_words_needs_a_key() {
    let world = World::new();
    assert_unauthenticated(&world, "POST", "/guardrails/validate_blocked_words_file").await;
}

#[tokio::test]
async fn apply_returns_the_first_returned_text() {
    let mut world = World::new();
    world.runner = RunnerMode::Texts(vec!["first".to_owned(), "second".to_owned()]);
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"first"}"#)
    );
    let call = runner_call(&world.events());
    assert_eq!(call.inputs.texts, Some(vec!["hello".to_owned()]));
}

#[tokio::test]
async fn apply_serves_the_alias_path() {
    let world = World::new();
    let response = world
        .send(request(
            "POST",
            "/apply_guardrail",
            r#"{"guardrail_name":"safety","text":"hello"}"#,
            true,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
}

#[tokio::test]
async fn apply_falls_back_to_the_request_text_without_texts() {
    let mut world = World::new();
    world.runner = RunnerMode::NoTexts;
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
}

#[tokio::test]
async fn apply_falls_back_to_the_request_text_with_an_empty_list() {
    let mut world = World::new();
    world.runner = RunnerMode::EmptyTexts;
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
}

#[tokio::test]
async fn apply_answers_the_proxy_404_for_an_unknown_guardrail() {
    let mut world = World::new();
    world.present = Arc::new(AtomicBool::new(false));
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"Guardrail 'safety' not found. Please ensure the guardrail is configured in your LiteLLM proxy.","type":"internal_server_error","param":null,"code":"404"}}"#
        )
    );
    assert_eq!(world.lookups.load(Ordering::SeqCst), 1);
    assert_eq!(world.resyncs.load(Ordering::SeqCst), 1);
    let events = world.events();
    assert_eq!(kinds(&events), UNKNOWN_GUARDRAIL_KINDS);
    assert_eq!(
        post_call_failure(&events).0,
        "Guardrail 'safety' not found. Please ensure the guardrail is configured in your LiteLLM proxy."
    );
}

#[tokio::test]
async fn apply_loads_the_guardrail_through_the_read_through() {
    let mut world = World::new();
    world.present = Arc::new(AtomicBool::new(false));
    world.resync_finds = true;
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
    assert_eq!(world.lookups.load(Ordering::SeqCst), 2);
    assert_eq!(world.resyncs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn apply_sends_only_the_keys_the_request_carried() {
    let world = World::new();
    let body = r#"{"guardrail_name":"safety","text":"hello","metadata":{"trace":"abc"}}"#;
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let call = runner_call(&world.events());
    assert_eq!(
        call.request_data,
        body_map(r#"{"metadata":{"trace":"abc"}}"#)
    );
}

#[tokio::test]
async fn apply_sends_an_empty_messages_list() {
    let world = World::new();
    let body = r#"{"guardrail_name":"safety","text":"hello","messages":[]}"#;
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let call = runner_call(&world.events());
    assert_eq!(call.request_data, body_map(r#"{"messages":[]}"#));
}

#[tokio::test]
async fn apply_pre_call_receives_the_data_dict() {
    let world = World::new();
    let body =
        r#"{"guardrail_name":"safety","text":"hello","messages":[{"role":"user","content":"hi"}]}"#;
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let (call_type, data) = pre_call(&world.events());
    assert_eq!(call_type, "apply_guardrail");
    assert_eq!(
        data,
        body_map(
            r#"{"guardrail_name":"safety","input":["hello"],"messages":[{"role":"user","content":"hi"}],"metadata":{"route":"/apply_guardrail"}}"#
        )
    );
}

#[tokio::test]
async fn apply_answers_422_for_a_missing_text() {
    let world = World::new();
    let response = apply_request(&world, r#"{"guardrail_name":"safety"}"#).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":[{"type":"missing","loc":["body","text"],"msg":"Field required"}]}"#)
    );
}

#[tokio::test]
async fn apply_answers_422_for_a_malformed_body() {
    let world = World::new();
    let response = apply_request(&world, r#"{"guardrail_name":"#).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error"}]}"#)
    );
}

#[rstest]
#[case::empty("")]
#[case::null("null")]
#[case::whitespace("  ")]
#[tokio::test]
async fn apply_answers_422_for_an_empty_body(#[case] body: &str) {
    let world = World::new();
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body_value(response).await,
        value(r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required"}]}"#),
        "{body}"
    );
}

#[tokio::test]
async fn apply_answers_the_proxy_error_body_for_a_guardrail_failure() {
    let mut world = World::new();
    world.runner = RunnerMode::HttpFailure {
        status: 403,
        detail: "blocked".to_owned(),
    };
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"blocked","type":"internal_server_error","param":null,"code":"403"}}"#
        )
    );
    let events = world.events();
    assert_eq!(kinds(&events), UNLOGGED_FAILURE_KINDS);
    assert_eq!(post_call_failure(&events).0, "blocked");
}

#[tokio::test]
async fn apply_records_allow_when_the_returned_text_is_unchanged() {
    let world = World::new();
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    let record = sink_record(&world.events());
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
    assert_eq!(record.guardrail_name.as_deref(), Some("safety"));
    assert_eq!(record.guardrail_response, text("allow"));
}

#[tokio::test]
async fn apply_needs_a_key() {
    let world = World::new();
    assert_unauthenticated(&world, "POST", "/guardrails/apply_guardrail").await;
    assert_unauthenticated(&world, "POST", "/apply_guardrail").await;
}

#[test]
fn input_type_upgrades_request_for_a_post_call_hook() {
    let hook = LitellmParamsMode::One("post_call".to_owned());
    assert_eq!(
        resolve(Some(&hook), "request"),
        GuardrailInputType::Response
    );
}

#[test]
fn input_type_keeps_request_for_other_hooks() {
    let pre_call_hook = LitellmParamsMode::One("pre_call".to_owned());
    let many_hook = LitellmParamsMode::Many(vec!["post_call".to_owned()]);
    assert_eq!(
        resolve(Some(&pre_call_hook), "request"),
        GuardrailInputType::Request
    );
    assert_eq!(
        resolve(Some(&many_hook), "request"),
        GuardrailInputType::Request
    );
    assert_eq!(resolve(None, "request"), GuardrailInputType::Request);
}

#[test]
fn input_type_keeps_response() {
    let hook = LitellmParamsMode::One("post_call".to_owned());
    assert_eq!(
        resolve(Some(&hook), "response"),
        GuardrailInputType::Response
    );
    assert_eq!(resolve(None, "response"), GuardrailInputType::Response);
}

#[test]
fn input_type_maps_an_unknown_type_to_request() {
    let hook = LitellmParamsMode::One("post_call".to_owned());
    assert_eq!(resolve(Some(&hook), "foo"), GuardrailInputType::Request);
    assert_eq!(resolve(Some(&hook), "Request"), GuardrailInputType::Request);
    assert_eq!(resolve(None, ""), GuardrailInputType::Request);
}

#[tokio::test]
async fn input_type_records_the_call_inside_the_invocation_window() {
    let mut world = World::new();
    world.event_hook = Some(LitellmParamsMode::One("post_call".to_owned()));
    world.runner = RunnerMode::Texts(vec!["masked".to_owned()]);
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    let events = world.events();
    assert_eq!(kinds(&events), UNLOGGED_SUCCESS_KINDS);
    let call = runner_call(&events);
    assert_eq!(call.input_type, GuardrailInputType::Response);
    let record = sink_record(&events);
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
    assert_eq!(record.guardrail_response, text("mask"));
    let start = record.start_time.unwrap();
    let end = record.end_time.unwrap();
    let at = call.at.duration_since(UNIX_EPOCH).unwrap().as_secs_f64();
    assert!((start..=end).contains(&at), "{start} <= {at} <= {end}");
}

#[tokio::test]
async fn logging_marks_the_pass_through_call_and_messages() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    let body =
        r#"{"guardrail_name":"safety","text":"hello","messages":[{"role":"user","content":"hi"}]}"#;
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let events = world.events();
    let expected: Vec<OpaqueParams> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap();
    assert_eq!(marked_messages(&events), expected);
    assert_eq!(
        runner_call(&events).request_data,
        body_map(r#"{"messages":[{"role":"user","content":"hi"}]}"#)
    );
}

#[tokio::test]
async fn logging_falls_back_to_one_user_turn_when_messages_are_empty() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    let body = r#"{"guardrail_name":"safety","text":"hello","messages":[]}"#;
    let response = apply_request(&world, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let events = world.events();
    let expected: Vec<OpaqueParams> =
        serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap();
    assert_eq!(marked_messages(&events), expected);
    assert_eq!(
        runner_call(&events).request_data,
        body_map(r#"{"messages":[]}"#)
    );
    let (_, data) = post_call_success(&events);
    assert_eq!(
        data.get("messages"),
        Some(&GuardrailValue::Array(Vec::new()))
    );
}

#[tokio::test]
async fn logging_emits_success_handlers_in_order_with_the_logged_payload() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.runner = RunnerMode::Texts(vec!["first".to_owned()]);
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"first"}"#)
    );
    let events = world.events();
    assert_eq!(kinds(&events), SUCCESS_LOG_KINDS);
    let logged = async_success(&events);
    let submitted = submit_success(&events);
    assert_eq!(logged.payload.response.response_text, "first");
    assert_eq!(submitted.payload, logged.payload);
    assert!(!logged.cache_hit);
    assert_eq!(logged.end_time, submitted.end_time);
    assert!(logged.start_time <= logged.end_time);
    let (hook_text, data) = post_call_success(&events);
    assert_eq!(hook_text, "first");
    assert_eq!(
        data,
        body_map(
            r#"{"guardrail_name":"safety","input":["hello"],"messages":[],"metadata":{"route":"/apply_guardrail"}}"#
        )
    );
}

#[tokio::test]
async fn logging_uses_the_replacement_success_response() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.success_hook = SuccessHook::Replace("replaced".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"replaced"}"#)
    );
    let events = world.events();
    assert_eq!(post_call_success(&events).0, "hello");
    assert_eq!(
        async_success(&events).payload.response.response_text,
        "replaced"
    );
}

#[tokio::test]
async fn logging_ignores_a_failing_success_hook() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.success_hook = SuccessHook::Fail("boom".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
    assert_eq!(kinds(&world.events()), SUCCESS_LOG_KINDS);
}

#[tokio::test]
async fn logging_ignores_failing_success_handlers() {
    let mut world = World::new();
    world.log = Some(LogMode::FailSuccessHandlers);
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_value(response).await,
        value(r#"{"response_text":"hello"}"#)
    );
    assert_eq!(kinds(&world.events()), SUCCESS_LOG_KINDS);
}

#[tokio::test]
async fn logging_skips_failure_handlers_for_an_http_error_but_runs_the_hook() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.runner = RunnerMode::HttpFailure {
        status: 403,
        detail: "blocked".to_owned(),
    };
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let events = world.events();
    assert_eq!(kinds(&events), HTTP_FAILURE_LOG_KINDS);
    assert_eq!(post_call_failure(&events).0, "blocked");
}

#[tokio::test]
async fn logging_runs_failure_handlers_for_other_errors() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.runner = RunnerMode::OtherFailure("boom".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"boom","type":"internal_server_error","param":null,"code":"500"}}"#
        )
    );
    let events = world.events();
    assert_eq!(kinds(&events), OTHER_FAILURE_LOG_KINDS);
    assert_eq!(async_failure(&events), "boom");
    assert_eq!(submit_failure(&events), "boom");
    assert_eq!(post_call_failure(&events).0, "boom");
}

#[tokio::test]
async fn logging_ignores_failing_failure_handlers() {
    let mut world = World::new();
    world.log = Some(LogMode::FailFailureHandlers);
    world.runner = RunnerMode::OtherFailure("boom".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"boom","type":"internal_server_error","param":null,"code":"500"}}"#
        )
    );
    assert_eq!(async_failure(&world.events()), "boom");
}

#[tokio::test]
async fn logging_ignores_a_failing_failure_hook() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.failure_hook = FailureHook::Fail("hook down".to_owned());
    world.runner = RunnerMode::OtherFailure("boom".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"boom","type":"internal_server_error","param":null,"code":"500"}}"#
        )
    );
    assert_eq!(post_call_failure(&world.events()).0, "boom");
}

#[tokio::test]
async fn logging_uses_the_failure_hook_replacement() {
    let mut world = World::new();
    world.log = Some(LogMode::default());
    world.failure_hook = FailureHook::Replace {
        status: 400,
        detail: "replaced by hook".to_owned(),
    };
    world.runner = RunnerMode::OtherFailure("boom".to_owned());
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"replaced by hook","type":"internal_server_error","param":null,"code":"400"}}"#
        )
    );
}

#[tokio::test]
async fn logging_fails_the_request_when_the_pass_through_patch_fails() {
    let mut world = World::new();
    world.log = Some(LogMode::FailMark);
    let response = apply_request(&world, r#"{"guardrail_name":"safety","text":"hello"}"#).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_value(response).await,
        value(
            r#"{"error":{"message":"the guardrail's logging handle is not accepting calls","type":"internal_server_error","param":null,"code":"500"}}"#
        )
    );
    assert_eq!(kinds(&world.events()), PATCH_FAILURE_KINDS);
}

#[rstest]
fn replays_the_blocked_words_file_corpus(
    #[files("fuzz/corpus/blocked_words_file/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(validate_blocked_words(&bytes));
}
