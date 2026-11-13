#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::HeaderMap as AxumHeaderMap;
use axum::http::{StatusCode as AxumStatusCode, Uri};
use axum::response::IntoResponse as _;
use axum::routing::post;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::alice_conduct_decisions::{
    Raw, UnreachableFallback as ConfigFallback,
};
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::error::AliceError;
use super::json_safe::{MAX_ITEMS, json_safe, safe_payload};
use super::{
    AliceConfigModel, AliceGuardrail, AliceGuardrailParams, DEFAULT_API_BASE,
    DEFAULT_BLOCK_MESSAGE, EVALUATE_PATH, GUARDRAIL_NAME,
};
use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

struct Recorded {
    path: String,
    af_api_key: Option<String>,
    content_type: Option<String>,
    body: serde_json::Value,
}

type Recorder = Arc<Mutex<Vec<Recorded>>>;

#[derive(Clone)]
enum Answer {
    Json {
        status: u16,
        body: serde_json::Value,
    },
    Raw {
        status: u16,
        body: &'static str,
    },
    Timeout,
}

struct Harness {
    base: String,
    recorded: Recorder,
    _tasks: JoinSet<io::Result<()>>,
}

async fn serve(answer: Answer) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("a listener address")
    );
    let recorded: Recorder = Arc::new(Mutex::new(Vec::new()));
    let handler_recorder = Arc::clone(&recorded);
    let handler = move |uri: Uri, headers: AxumHeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&handler_recorder);
        let answer = answer.clone();
        async move {
            let entry = Recorded {
                path: uri.path().to_owned(),
                af_api_key: header_text(&headers, "af-api-key"),
                content_type: header_text(&headers, "content-type"),
                body: serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            };
            recorder.lock().expect("the recorder").push(entry);
            match answer {
                Answer::Json { status, body } => (
                    AxumStatusCode::from_u16(status).expect("a status code"),
                    axum::Json(body),
                )
                    .into_response(),
                Answer::Raw { status, body } => (
                    AxumStatusCode::from_u16(status).expect("a status code"),
                    body,
                )
                    .into_response(),
                Answer::Timeout => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    (AxumStatusCode::OK, "late").into_response()
                }
            }
        }
    };
    let router = Router::new().route(EVALUATE_PATH, post(handler));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Harness {
        base,
        recorded,
        _tasks: tasks,
    }
}

fn header_text(headers: &AxumHeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn empty_env(_name: &str) -> String {
    String::new()
}

fn environment(name: &str) -> String {
    match name {
        "ALICE_API_KEY" => "env-alice-key".to_owned(),
        "ALICE_API_BASE" => "https://env.alice.example/".to_owned(),
        _ => String::new(),
    }
}

fn guardrail(base: &str, fallback: UnreachableFallback, timeout: Duration) -> AliceGuardrail {
    AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: Some(SecretString::from("alice-key")),
            api_base: Some(base.to_owned()),
            unreachable_fallback: Some(fallback),
        },
        empty_env,
        reqwest::Client::default(),
        timeout,
    )
    .expect("the guardrail builds")
}

fn empty_inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn text_inputs(values: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(values.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

enum Outcome {
    Returned(GuardrailApiInputs),
    Failed(GuardrailFailure),
}

async fn apply(
    guardrail: &AliceGuardrail,
    inputs: GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Outcome {
    let mut data = request_data.clone();
    match <AliceGuardrail as GuardrailContract>::apply_guardrail(
        guardrail,
        inputs,
        &mut data,
        GuardrailInputType::Request,
        None,
    )
    .await
    {
        Ok(inputs) => Outcome::Returned(inputs),
        Err(failure) => Outcome::Failed(failure),
    }
}

fn failure(outcome: Outcome) -> GuardrailFailure {
    match outcome {
        Outcome::Returned(_inputs) => panic!("expected the guardrail to refuse"),
        Outcome::Failed(failure) => failure,
    }
}

fn returned_inputs(outcome: Outcome) -> GuardrailApiInputs {
    match outcome {
        Outcome::Returned(inputs) => inputs,
        Outcome::Failed(failure) => panic!("expected the guardrail to pass, got {failure}"),
    }
}

fn raised(outcome: Outcome) -> (String, u16, bool) {
    match failure(outcome) {
        GuardrailFailure::GuardrailRaised {
            guardrail_name,
            message,
            status_code,
            blocked_content,
            ..
        } => (
            guardrail_name.unwrap_or_default(),
            status_code,
            blocked_content,
        ),
        other => panic!("expected a guardrail-raised failure, got {other}"),
    }
}

fn answer(body: serde_json::Value) -> Answer {
    Answer::Json { status: 200, body }
}

#[tokio::test]
async fn alice_guardrail_empty_inputs_skip_the_call() {
    let harness = serve(answer(serde_json::json!({"verdict": "BLOCK"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, empty_inputs(), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), empty_inputs());
    assert!(
        harness.recorded.lock().expect("the recorder").is_empty(),
        "an empty batch never reaches Alice"
    );
}

#[rstest]
#[case::empty_texts(GuardrailApiInputs {
    texts: Some(Vec::new()),
    images: None,
    tools: None,
    tool_calls: None,
    structured_messages: None,
    model: Some("gpt".to_owned()),
    stream_holdback_chars: None,
})]
#[case::only_a_model(GuardrailApiInputs {
    texts: None,
    images: None,
    tools: None,
    tool_calls: None,
    structured_messages: None,
    model: Some("gpt".to_owned()),
    stream_holdback_chars: Some(vec![1]),
})]
#[tokio::test]
async fn alice_guardrail_fields_outside_the_selectable_five_do_not_start_a_call(
    #[case] inputs: GuardrailApiInputs,
) {
    let harness = serve(answer(serde_json::json!({"verdict": "ALLOW"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, inputs.clone(), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), inputs);
    assert!(harness.recorded.lock().expect("the recorder").is_empty());
}

#[tokio::test]
async fn alice_guardrail_posts_the_stripped_bounded_payload() {
    let harness = serve(answer(serde_json::json!({"verdict": "ALLOW"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let request_data = BTreeMap::from([
        (
            "api_key".to_owned(),
            GuardrailValue::Text("sk-secret".to_owned()),
        ),
        (
            "metadata".to_owned(),
            GuardrailValue::Object(BTreeMap::from([
                (
                    "headers".to_owned(),
                    GuardrailValue::Object(BTreeMap::from([(
                        "authorization".to_owned(),
                        GuardrailValue::Text("Bearer secret".to_owned()),
                    )])),
                ),
                ("keep".to_owned(), GuardrailValue::SignedInteger(1)),
            ])),
        ),
        (
            "secret_fields".to_owned(),
            GuardrailValue::Object(BTreeMap::new()),
        ),
        (
            "list".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
                (
                    "raw_headers".to_owned(),
                    GuardrailValue::Object(BTreeMap::new()),
                ),
                ("ok".to_owned(), GuardrailValue::Boolean(true)),
            ]))]),
        ),
    ]);
    let outcome = apply(&instance, text_inputs(&["hello"]), &request_data).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["hello"]));
    let recorded = harness.recorded.lock().expect("the recorder");
    assert_eq!(recorded.len(), 1);
    let request = &recorded[0];
    assert_eq!(request.path, EVALUATE_PATH);
    assert_eq!(request.af_api_key.as_deref(), Some("alice-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    assert_eq!(request.body["input_type"], "request");
    assert_eq!(request.body["inputs"]["texts"][0], "hello");
    let sent = &request.body["request_data"];
    assert!(sent.get("api_key").is_none());
    assert!(sent.get("secret_fields").is_none());
    assert_eq!(sent["metadata"]["keep"], 1);
    assert!(sent["metadata"].get("headers").is_none());
    assert_eq!(sent["list"][0]["ok"], true);
    assert!(sent["list"][0].get("raw_headers").is_none());
}

#[tokio::test]
async fn alice_guardrail_allow_returns_the_inputs_unchanged() {
    let harness = serve(answer(serde_json::json!({"verdict": "ALLOW"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["keep me"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["keep me"]));
}

#[tokio::test]
async fn alice_guardrail_block_raises_400_with_the_verdict_message() {
    let harness = serve(answer(
        serde_json::json!({"verdict": "BLOCK", "message": "policy says no"}),
    ))
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let (name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["bad"]), &BTreeMap::new()).await);
    assert_eq!(name, GUARDRAIL_NAME);
    assert_eq!(status, 400);
    assert!(blocked_content);
}

#[tokio::test]
async fn alice_guardrail_block_without_a_message_uses_the_default_text() {
    let harness = serve(answer(serde_json::json!({"verdict": "BLOCK"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    match failure(apply(&instance, text_inputs(&["bad"]), &BTreeMap::new()).await) {
        GuardrailFailure::GuardrailRaised { message, .. } => {
            assert_eq!(message, DEFAULT_BLOCK_MESSAGE);
        }
        other => panic!("expected a guardrail-raised failure, got {other}"),
    }
}

#[tokio::test]
async fn alice_guardrail_mask_applies_replacements_in_order() {
    let harness = serve(answer(serde_json::json!({
        "verdict": "MASK",
        "replacements": [
            {"index": 0, "text": "first"},
            {"index": 1, "text": "second"},
            {"index": 0, "text": "last"},
        ],
    })))
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["a", "b"]), &BTreeMap::new()).await;
    assert_eq!(
        returned_inputs(outcome).texts,
        Some(vec!["last".to_owned(), "second".to_owned()])
    );
}

#[rstest]
#[case::out_of_range(serde_json::json!({"verdict": "MASK", "replacements": [{"index": 2, "text": "x"}]}))]
#[case::missing_text(serde_json::json!({"verdict": "MASK", "replacements": [{"index": 0}]}))]
#[case::missing_index(serde_json::json!({"verdict": "MASK", "replacements": [{"text": "x"}]}))]
#[case::negative_index(serde_json::json!({"verdict": "MASK", "replacements": [{"index": -1, "text": "x"}]}))]
#[case::no_replacements(serde_json::json!({"verdict": "MASK"}))]
#[tokio::test]
async fn alice_guardrail_mask_refuses_a_malformed_verdict(#[case] body: serde_json::Value) {
    let harness = serve(answer(body)).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let (name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["a"]), &BTreeMap::new()).await);
    assert_eq!(name, GUARDRAIL_NAME);
    assert_eq!(status, 400);
    assert!(blocked_content);
}

#[tokio::test]
async fn alice_guardrail_mask_without_texts_cannot_apply_a_replacement() {
    let harness = serve(answer(serde_json::json!({
        "verdict": "MASK",
        "replacements": [{"index": 0, "text": "x"}],
    })))
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let inputs = GuardrailApiInputs {
        images: Some(vec!["data:image/png;base64,aGk=".to_owned()]),
        texts: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let (_name, status, blocked_content) = raised(apply(&instance, inputs, &BTreeMap::new()).await);
    assert_eq!(status, 400);
    assert!(blocked_content);
}

#[tokio::test]
async fn alice_guardrail_detect_logs_the_correlation_id_and_categories() {
    let harness = serve(answer(serde_json::json!({
        "verdict": "DETECT",
        "correlation_id": "corr-1",
        "categories": ["cat-a", "cat-b"],
    })))
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let outcome = apply(&instance, text_inputs(&["watch me"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["watch me"]));
    let text = captured.text();
    assert!(
        text.contains("corr-1"),
        "expected the correlation id in {text}"
    );
    assert!(text.contains("cat-a"), "expected the categories in {text}");
}

#[tokio::test]
async fn alice_guardrail_unrecognized_verdict_fails_closed() {
    let harness = serve(answer(serde_json::json!({"verdict": "REVIEW"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let (name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await);
    assert_eq!(name, GUARDRAIL_NAME);
    assert_eq!(status, 400);
    assert!(!blocked_content);
}

#[tokio::test]
async fn alice_guardrail_missing_verdict_fails_open_when_configured() {
    let harness = serve(answer(serde_json::json!({"message": "no verdict"}))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
}

#[rstest]
#[case::server_error(Answer::Json { status: 500, body: serde_json::json!({}) })]
#[case::bad_gateway(Answer::Json { status: 502, body: serde_json::json!({}) })]
#[case::decode_error(Answer::Raw { status: 200, body: "{not json" })]
#[case::non_object_body(Answer::Json { status: 200, body: serde_json::json!("a string") })]
#[case::malformed_verdict(Answer::Json { status: 200, body: serde_json::json!({"verdict": 7}) })]
#[tokio::test]
async fn alice_guardrail_outages_fail_closed_by_default(#[case] scripted: Answer) {
    let harness = serve(scripted).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let (name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await);
    assert_eq!(name, GUARDRAIL_NAME);
    assert_eq!(status, 400, "the default policy refuses the request");
    assert!(!blocked_content);
}

#[rstest]
#[case::server_error(Answer::Json { status: 503, body: serde_json::json!({}) })]
#[case::malformed_verdict(Answer::Json { status: 200, body: serde_json::json!({"verdict": 7}) })]
#[tokio::test]
async fn alice_guardrail_outages_fail_open_when_configured(#[case] scripted: Answer) {
    let harness = serve(scripted).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
}

#[tokio::test]
async fn alice_guardrail_timeout_follows_the_policy() {
    let harness = serve(Answer::Timeout).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_millis(20),
    );
    let (_name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await);
    assert_eq!(status, 400);
    assert!(!blocked_content);
}

#[tokio::test]
async fn alice_guardrail_connection_failure_follows_the_policy() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("a listener address")
    );
    drop(listener);
    let instance = guardrail(&base, UnreachableFallback::FailOpen, Duration::from_secs(2));
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
}

#[tokio::test]
async fn alice_guardrail_fail_open_logs_at_error_level() {
    let harness = serve(Answer::Json {
        status: 500,
        body: serde_json::json!({}),
    })
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
    assert!(
        captured.text().contains("Alice guardrail unreachable"),
        "the outage is logged at critical level"
    );
}

#[tokio::test]
async fn alice_guardrail_builds_the_url_from_a_base_with_a_trailing_slash() {
    let harness = serve(answer(serde_json::json!({"verdict": "ALLOW"}))).await;
    let base = format!("{}/", harness.base);
    let instance = guardrail(
        &base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    assert_eq!(
        instance.evaluate_url,
        format!("{}{EVALUATE_PATH}", harness.base)
    );
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
    assert_eq!(
        harness.recorded.lock().expect("the recorder")[0].path,
        EVALUATE_PATH
    );
}

#[test]
fn alice_guardrail_missing_api_key_raises_missing_secrets() {
    let error = AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: None,
            api_base: None,
            unreachable_fallback: None,
        },
        empty_env,
        reqwest::Client::default(),
        Duration::from_secs(2),
    )
    .expect_err("a missing key refuses to build");
    assert!(matches!(error, AliceError::MissingSecrets));
    assert!(error.to_string().contains("ALICE_API_KEY"));
}

#[test]
fn alice_guardrail_an_empty_api_key_raises_missing_secrets() {
    let error = AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: Some(SecretString::from(String::new())),
            api_base: None,
            unreachable_fallback: None,
        },
        empty_env,
        reqwest::Client::default(),
        Duration::from_secs(2),
    )
    .expect_err("an empty key refuses to build");
    assert!(matches!(error, AliceError::MissingSecrets));
}

#[test]
fn alice_guardrail_reads_the_key_and_base_from_the_environment() {
    let instance = AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: None,
            api_base: None,
            unreachable_fallback: None,
        },
        environment,
        reqwest::Client::default(),
        Duration::from_secs(2),
    )
    .expect("the environment supplies both values");
    assert_eq!(instance.api_key.expose_secret(), "env-alice-key");
    assert_eq!(
        instance.evaluate_url,
        "https://env.alice.example/v2/evaluate/litellm"
    );
    assert_eq!(
        instance.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn alice_guardrail_defaults_api_base_and_fallback() {
    let instance = AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: Some(SecretString::from("key")),
            api_base: None,
            unreachable_fallback: None,
        },
        empty_env,
        reqwest::Client::default(),
        Duration::from_secs(2),
    )
    .expect("an explicit key is enough");
    assert_eq!(
        instance.evaluate_url,
        format!("{DEFAULT_API_BASE}{EVALUATE_PATH}")
    );
    assert_eq!(
        instance.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
}

#[test]
fn alice_guardrail_config_model_deserializes_the_litellm_fields() {
    let model: AliceConfigModel = serde_json::from_str(r#"{"api_base": "https://alice.example"}"#)
        .expect("the config model accepts an api_base");
    assert!(model.api_key.is_none());
    assert_eq!(model.api_base.as_deref(), Some("https://alice.example"));
    assert_eq!(AliceConfigModel::ui_friendly_name(), "Alice");
    let with_key: AliceConfigModel = serde_json::from_str(r#"{"api_key": "secret"}"#)
        .expect("the config model accepts an api_key");
    assert_eq!(
        with_key
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("secret")
    );
}

#[test]
fn alice_guardrail_config_section_loads_values_and_environment() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
  - guardrail_name: not-alice
    litellm_params:
      guardrail: bedrock
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let environment = BTreeMap::from([
        ("ALICE_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "ALICE_API_BASE".to_owned(),
            "https://env.example".to_owned(),
        ),
    ]);
    let settings = raw.convert(&environment).expect("the section converts");
    assert_eq!(settings.guardrails.len(), 1);
    let configured = &settings.guardrails[0];
    assert_eq!(configured.guardrail_name, "alice-check");
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("env-key")
    );
    assert_eq!(configured.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(configured.unreachable_fallback, ConfigFallback::FailClosed);
}

#[test]
fn alice_guardrail_config_section_prefers_config_and_defaults_are_litellm_s() {
    let yaml = r"
guardrails:
  - guardrail_name: alice-check
    litellm_params:
      guardrail: alice
      api_key: config-key
      unreachable_fallback: fail_open
";
    let raw: Raw = serde_saphyr::from_str(yaml).expect("the section parses");
    let settings = raw.convert(&BTreeMap::new()).expect("the section converts");
    let configured = &settings.guardrails[0];
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("config-key")
    );
    assert_eq!(configured.api_base, None);
    assert_eq!(configured.unreachable_fallback, ConfigFallback::FailOpen);
    assert_eq!(DEFAULT_API_BASE, "https://api.alice.io");
}

#[test]
fn alice_guardrail_json_safe_drops_a_container_at_the_depth_limit() {
    let value = nested(
        12,
        GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]),
    );
    assert_eq!(safe_payload(&value, &[]), nested(12, GuardrailValue::Null));
}

#[test]
fn alice_guardrail_json_safe_keeps_a_container_just_inside_the_limit() {
    let value = nested(
        11,
        GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]),
    );
    assert_eq!(safe_payload(&value, &[]), value);
}

#[test]
fn alice_guardrail_json_safe_drops_a_revisited_value() {
    let value = GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]);
    let mut seen = vec![std::ptr::from_ref(&value)];
    assert_eq!(json_safe(&value, 0, &[], &mut seen), GuardrailValue::Null);
}

#[test]
fn alice_guardrail_json_safe_caps_each_container_at_five_thousand_items() {
    let items = (0..5001)
        .map(GuardrailValue::SignedInteger)
        .collect::<Vec<_>>();
    let kept = safe_payload(&GuardrailValue::Array(items), &[]);
    match kept {
        GuardrailValue::Array(values) => {
            assert_eq!(values.len(), MAX_ITEMS);
            assert_eq!(values.last(), Some(&GuardrailValue::SignedInteger(4999)));
        }
        other => panic!("expected an array, got {other:?}"),
    }
}

#[test]
fn alice_guardrail_json_safe_strips_credential_keys_at_any_depth() {
    let value = GuardrailValue::Object(BTreeMap::from([
        (
            "outer".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
                (
                    "headers".to_owned(),
                    GuardrailValue::Text("dropped".to_owned()),
                ),
                (
                    "provider_specific_header".to_owned(),
                    GuardrailValue::Text("dropped".to_owned()),
                ),
                ("keep".to_owned(), GuardrailValue::Text("kept".to_owned())),
            ]))]),
        ),
        (
            "headers".to_owned(),
            GuardrailValue::Text("dropped".to_owned()),
        ),
    ]));
    let stripped = safe_payload(&value, &super::json_safe::CREDENTIAL_KEYS_TO_STRIP);
    match stripped {
        GuardrailValue::Object(fields) => {
            assert!(fields.get("headers").is_none());
            match fields.get("outer") {
                Some(GuardrailValue::Array(items)) => match items.first() {
                    Some(GuardrailValue::Object(inner)) => {
                        assert!(inner.get("headers").is_none());
                        assert!(inner.get("provider_specific_header").is_none());
                        assert_eq!(
                            inner.get("keep"),
                            Some(&GuardrailValue::Text("kept".to_owned()))
                        );
                    }
                    other => panic!("expected an object, got {other:?}"),
                },
                other => panic!("expected an array, got {other:?}"),
            }
        }
        other => panic!("expected an object, got {other:?}"),
    }
}

fn nested(depth: usize, leaf: GuardrailValue) -> GuardrailValue {
    let mut value = leaf;
    for _ in 0..depth {
        value = GuardrailValue::Array(vec![value]);
    }
    value
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the log buffer").clone()).expect("the log is UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
