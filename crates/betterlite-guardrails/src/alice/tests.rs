#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::HeaderMap as AxumHeaderMap;
use axum::http::{StatusCode as AxumStatusCode, Uri, header};
use axum::response::{IntoResponse as _, Response as AxumResponse};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::subscriber::set_default;

use super::error::AliceError;
use super::transport::MAX_RESPONSE_BYTES;
use super::{
    AliceGuardrail, AliceGuardrailParams, DEFAULT_API_BASE, DEFAULT_BLOCK_MESSAGE, EVALUATE_PATH,
    GUARDRAIL_NAME,
};
use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam, UnreachableFallback,
};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

mod config_section;

#[derive(Clone)]
struct Recorded {
    path: String,
    af_api_key: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct SentPayload {
    input_type: GuardrailInputType,
    inputs: GuardrailApiInputs,
    request_data: BTreeMap<String, GuardrailValue>,
}

type Recorder = Arc<Mutex<Vec<Recorded>>>;

#[derive(Clone)]
enum Answer {
    Json { status: u16, body: String },
    Raw { status: u16, body: String },
    Timeout,
}

#[derive(Serialize)]
struct ScriptedVerdict {
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replacements: Option<Vec<ScriptedReplacement>>,
}

#[derive(Serialize)]
struct ScriptedReplacement {
    index: i64,
    text: String,
}

fn verdict(name: &str) -> ScriptedVerdict {
    ScriptedVerdict {
        verdict: Some(name.to_owned()),
        message: None,
        correlation_id: None,
        categories: None,
        replacements: None,
    }
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
        let shared = Arc::clone(&handler_recorder);
        let scripted = answer.clone();
        async move {
            let entry = Recorded {
                path: uri.path().to_owned(),
                af_api_key: header_text(&headers, "af-api-key"),
                content_type: header_text(&headers, "content-type"),
                body: bytes.to_vec(),
            };
            shared.lock().expect("the recorder").push(entry);
            scripted_response(scripted).await
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

async fn scripted_response(answer: Answer) -> AxumResponse {
    match answer {
        Answer::Json { status, body } => (
            AxumStatusCode::from_u16(status).expect("a status code"),
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Answer::Raw { status, body } => (
            AxumStatusCode::from_u16(status).expect("a status code"),
            body,
        )
            .into_response(),
        Answer::Timeout => {
            sleep(Duration::from_millis(200)).await;
            (AxumStatusCode::OK, "late").into_response()
        }
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
        Ok(returned) => Outcome::Returned(returned),
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
            status_code,
            blocked_content,
            ..
        } => (
            guardrail_name.unwrap_or_default(),
            status_code,
            blocked_content,
        ),
        other @ (GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_)) => {
            panic!("expected a guardrail-raised failure, got {other}")
        }
    }
}

fn answer<T>(body: T) -> Answer
where
    T: Serialize,
{
    Answer::Json {
        status: 200,
        body: serde_json::to_string(&body).expect("the scripted answer serializes"),
    }
}

#[tokio::test]
async fn alice_guardrail_empty_inputs_skip_the_call() {
    let harness = serve(answer(verdict("BLOCK"))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, GuardrailApiInputs::default(), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), GuardrailApiInputs::default());
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
    let harness = serve(answer(verdict("ALLOW"))).await;
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
    let harness = serve(answer(verdict("ALLOW"))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["hello"]), &secret_request_data()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["hello"]));
    let request = {
        let recorded = harness.recorded.lock().expect("the recorder");
        assert_eq!(recorded.len(), 1);
        recorded[0].clone()
    };
    assert_eq!(request.path, EVALUATE_PATH);
    assert_eq!(request.af_api_key.as_deref(), Some("alice-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    assert_stripped_payload(&request.body);
}

fn secret_request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
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
    ])
}

fn assert_stripped_payload(body: &[u8]) {
    let sent: SentPayload = serde_json::from_slice(body).expect("the evaluation body is JSON");
    assert_eq!(sent.input_type, GuardrailInputType::Request);
    assert_eq!(sent.inputs.texts, Some(vec!["hello".to_owned()]));
    assert!(!sent.request_data.contains_key("api_key"));
    assert!(!sent.request_data.contains_key("secret_fields"));
    let Some(GuardrailValue::Object(metadata)) = sent.request_data.get("metadata") else {
        panic!("metadata is sent as an object");
    };
    assert_eq!(
        metadata.get("keep"),
        Some(&GuardrailValue::SignedInteger(1))
    );
    assert!(!metadata.contains_key("headers"));
    let Some(GuardrailValue::Array(list)) = sent.request_data.get("list") else {
        panic!("list is sent as an array");
    };
    let Some(GuardrailValue::Object(first)) = list.first() else {
        panic!("the first list item is an object");
    };
    assert_eq!(first.get("ok"), Some(&GuardrailValue::Boolean(true)));
    assert!(!first.contains_key("raw_headers"));
}

#[tokio::test]
async fn alice_guardrail_allow_returns_the_inputs_unchanged() {
    let harness = serve(answer(verdict("ALLOW"))).await;
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
    let harness = serve(answer(ScriptedVerdict {
        verdict: Some("BLOCK".to_owned()),
        message: Some("policy says no".to_owned()),
        correlation_id: None,
        categories: None,
        replacements: None,
    }))
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
    let harness = serve(answer(verdict("BLOCK"))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    match failure(apply(&instance, text_inputs(&["bad"]), &BTreeMap::new()).await) {
        GuardrailFailure::GuardrailRaised { message, .. } => {
            assert_eq!(message, DEFAULT_BLOCK_MESSAGE);
        }
        other @ (GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_)) => {
            panic!("expected a guardrail-raised failure, got {other}")
        }
    }
}

#[tokio::test]
async fn alice_guardrail_mask_applies_replacements_in_order() {
    let harness = serve(answer(ScriptedVerdict {
        verdict: Some("MASK".to_owned()),
        message: None,
        correlation_id: None,
        categories: None,
        replacements: Some(vec![
            ScriptedReplacement {
                index: 0,
                text: "first".to_owned(),
            },
            ScriptedReplacement {
                index: 1,
                text: "second".to_owned(),
            },
            ScriptedReplacement {
                index: 0,
                text: "last".to_owned(),
            },
        ]),
    }))
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
#[case::out_of_range(r#"{"verdict": "MASK", "replacements": [{"index": 2, "text": "x"}]}"#)]
#[case::missing_text(r#"{"verdict": "MASK", "replacements": [{"index": 0}]}"#)]
#[case::missing_index(r#"{"verdict": "MASK", "replacements": [{"text": "x"}]}"#)]
#[case::negative_index(r#"{"verdict": "MASK", "replacements": [{"index": -1, "text": "x"}]}"#)]
#[case::no_replacements(r#"{"verdict": "MASK"}"#)]
#[tokio::test]
async fn alice_guardrail_mask_refuses_a_malformed_verdict(#[case] body: &str) {
    let harness = serve(Answer::Json {
        status: 200,
        body: body.to_owned(),
    })
    .await;
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
    let harness = serve(answer(ScriptedVerdict {
        verdict: Some("MASK".to_owned()),
        message: None,
        correlation_id: None,
        categories: None,
        replacements: Some(vec![ScriptedReplacement {
            index: 0,
            text: "x".to_owned(),
        }]),
    }))
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
    let harness = serve(answer(ScriptedVerdict {
        verdict: Some("DETECT".to_owned()),
        message: None,
        correlation_id: Some("corr-1".to_owned()),
        categories: Some(vec!["cat-a".to_owned(), "cat-b".to_owned()]),
        replacements: None,
    }))
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
    let _guard = set_default(subscriber);
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
    let harness = serve(answer(verdict("REVIEW"))).await;
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
    let harness = serve(answer(ScriptedVerdict {
        verdict: None,
        message: Some("no verdict".to_owned()),
        correlation_id: None,
        categories: None,
        replacements: None,
    }))
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
}

#[tokio::test]
async fn alice_guardrail_a_redirect_propagates_instead_of_failing_open() {
    let harness = serve(Answer::Json {
        status: 304,
        body: "{}".to_owned(),
    })
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    match failure(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await) {
        GuardrailFailure::Http { status, .. } => assert_eq!(status, 304),
        other @ (GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_)) => panic!("expected an http failure, got {other}"),
    }
}

#[tokio::test]
async fn alice_guardrail_a_client_error_propagates_without_the_fallback_policy() {
    let harness = serve(Answer::Json {
        status: 401,
        body: "{}".to_owned(),
    })
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailOpen,
        Duration::from_secs(2),
    );
    match failure(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await) {
        GuardrailFailure::Http { status, .. } => assert_eq!(status, 401),
        other @ (GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_)) => panic!("expected an http failure, got {other}"),
    }
}

#[rstest]
#[case::server_error(Answer::Json { status: 500, body: "{}".to_owned() })]
#[case::bad_gateway(Answer::Json { status: 502, body: "{}".to_owned() })]
#[case::decode_error(Answer::Raw { status: 200, body: "{not json".to_owned() })]
#[case::non_object_body(Answer::Json { status: 200, body: r#""a string""#.to_owned() })]
#[case::malformed_verdict(Answer::Json { status: 200, body: r#"{"verdict": 7}"#.to_owned() })]
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
#[case::server_error(Answer::Json { status: 503, body: "{}".to_owned() })]
#[case::malformed_verdict(Answer::Json { status: 200, body: r#"{"verdict": 7}"#.to_owned() })]
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
        body: "{}".to_owned(),
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
    let _guard = set_default(subscriber);
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
    assert!(
        captured.text().contains("Alice guardrail unreachable"),
        "the outage is logged at critical level"
    );
}

#[tokio::test]
async fn alice_guardrail_builds_the_url_from_a_base_with_a_trailing_slash() {
    let harness = serve(answer(verdict("ALLOW"))).await;
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

#[rstest]
#[case::texts(text_inputs(&["hi"]))]
#[case::images(GuardrailApiInputs {
    texts: None,
    images: Some(vec!["data:image/png;base64,aGk=".to_owned()]),
    tools: None,
    tool_calls: None,
    structured_messages: None,
    model: None,
    stream_holdback_chars: None,
})]
#[case::tools(GuardrailApiInputs {
    texts: None,
    images: None,
    tools: Some(vec![GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    }]),
    tool_calls: None,
    structured_messages: None,
    model: None,
    stream_holdback_chars: None,
})]
#[case::tool_calls(GuardrailApiInputs {
    texts: None,
    images: None,
    tools: None,
    tool_calls: Some(vec![GuardrailValue::Object(BTreeMap::new())]),
    structured_messages: None,
    model: None,
    stream_holdback_chars: None,
})]
#[case::structured_messages(GuardrailApiInputs {
    texts: None,
    images: None,
    tools: None,
    tool_calls: None,
    structured_messages: Some(vec![GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text("hello".to_owned()),
        )]),
    }]),
    model: None,
    stream_holdback_chars: None,
})]
#[tokio::test]
async fn alice_guardrail_any_one_selectable_field_starts_the_call(
    #[case] inputs: GuardrailApiInputs,
) {
    let harness = serve(answer(verdict("ALLOW"))).await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(2),
    );
    let outcome = apply(&instance, inputs.clone(), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), inputs);
    assert_eq!(harness.recorded.lock().expect("the recorder").len(), 1);
}

#[test]
fn alice_guardrail_response_cap_is_one_mebibyte() {
    assert_eq!(MAX_RESPONSE_BYTES, 0x0010_0000);
}

#[tokio::test]
async fn alice_guardrail_reads_a_body_at_the_response_cap() {
    let harness = serve(Answer::Raw {
        status: 200,
        body: padded_verdict(MAX_RESPONSE_BYTES),
    })
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(5),
    );
    let outcome = apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await;
    assert_eq!(returned_inputs(outcome), text_inputs(&["x"]));
}

#[tokio::test]
async fn alice_guardrail_refuses_a_body_over_the_response_cap() {
    let harness = serve(Answer::Raw {
        status: 200,
        body: padded_verdict(MAX_RESPONSE_BYTES.saturating_add(1)),
    })
    .await;
    let instance = guardrail(
        &harness.base,
        UnreachableFallback::FailClosed,
        Duration::from_secs(5),
    );
    let (_name, status, blocked_content) =
        raised(apply(&instance, text_inputs(&["x"]), &BTreeMap::new()).await);
    assert_eq!(status, 400);
    assert!(!blocked_content);
}

#[test]
fn alice_guardrail_a_non_object_body_is_reported_as_such() {
    assert!(matches!(
        super::transport::verdict_from_body(b"\"a string\""),
        Err(AliceError::NonObjectBody)
    ));
}

fn padded_verdict(total: usize) -> String {
    let prefix = r#"{"verdict":"ALLOW","message":""#;
    let suffix = r#""}"#;
    let padding = total.saturating_sub(prefix.len().saturating_add(suffix.len()));
    format!("{prefix}{}{suffix}", "a".repeat(padding))
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the log buffer").clone()).expect("the log is UTF-8")
    }
}

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
