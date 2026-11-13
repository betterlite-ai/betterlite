#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::error::PromptGuardError;
use super::init::PromptGuardArgs;
use super::wire::{self, Direction, GuardResponse};
use super::{DEFAULT_API_BASE, MAX_RESPONSE_BYTES, PromptGuardGuardrail};
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::inputs::GuardrailApiInputs;

fn args(api_key: Option<&str>, block_on_error: Option<bool>) -> PromptGuardArgs {
    PromptGuardArgs {
        api_base: Some("https://guard.test".to_owned()),
        api_key: api_key.map(SecretString::from),
        block_on_error,
        timeout: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        name: "pg-test".to_owned(),
    }
}

fn with_base(mut args: PromptGuardArgs, base: Option<&str>) -> PromptGuardArgs {
    args.api_base = base.map(ToOwned::to_owned);
    args
}

fn client() -> reqwest::Client {
    base_builder().build().expect("client")
}

fn env(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn text_inputs() -> GuardrailApiInputs {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["hello".to_owned()]);
    inputs
}

fn built(inputs: GuardrailApiInputs, input_type: GuardrailInputType) -> Option<wire::GuardRequest> {
    wire::build_request(&inputs, input_type)
}

#[test]
fn missing_api_key_raises_the_credentials_error() {
    let error = PromptGuardGuardrail::new(args(None, None), &env(&[]), client()).unwrap_err();
    assert!(matches!(error, PromptGuardError::MissingCredentials));
    assert_eq!(
        error.to_string(),
        "PromptGuard API key is required. Set PROMPTGUARD_API_KEY in the environment or pass api_key in the guardrail config."
    );
}

#[test]
fn api_key_takes_the_argument_then_the_environment() {
    let from_argument = PromptGuardGuardrail::new(
        args(Some("arg-key"), None),
        &env(&[("PROMPTGUARD_API_KEY", "env-key")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(from_argument.api_key.expose_secret(), "arg-key");
    let from_environment = PromptGuardGuardrail::new(
        args(None, None),
        &env(&[("PROMPTGUARD_API_KEY", "env-key")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(from_environment.api_key.expose_secret(), "env-key");
    let empty_argument = PromptGuardGuardrail::new(
        args(Some(""), None),
        &env(&[("PROMPTGUARD_API_KEY", "env-key")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(empty_argument.api_key.expose_secret(), "env-key");
    let empty_environment = PromptGuardGuardrail::new(
        args(None, None),
        &env(&[("PROMPTGUARD_API_KEY", "")]),
        client(),
    )
    .unwrap_err();
    assert!(matches!(
        empty_environment,
        PromptGuardError::MissingCredentials
    ));
}

#[test]
fn api_base_takes_the_argument_then_the_environment_then_the_default() {
    let by_default =
        PromptGuardGuardrail::new(args(Some("key"), None), &env(&[]), client()).expect("guardrail");
    assert_eq!(by_default.api_base, DEFAULT_API_BASE);
    let from_environment = PromptGuardGuardrail::new(
        args(Some("key"), None),
        &env(&[("PROMPTGUARD_API_BASE", "https://env.test/")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(from_environment.api_base, "https://env.test");
    let from_argument = PromptGuardGuardrail::new(
        with_base(args(Some("key"), None), Some("https://arg.test//")),
        &env(&[("PROMPTGUARD_API_BASE", "https://env.test")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(from_argument.api_base, "https://arg.test");
    let empty_argument = PromptGuardGuardrail::new(
        with_base(args(Some("key"), None), Some("")),
        &env(&[("PROMPTGUARD_API_BASE", "https://env.test")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(empty_argument.api_base, "https://env.test");
    let empty_environment = PromptGuardGuardrail::new(
        args(Some("key"), None),
        &env(&[("PROMPTGUARD_API_BASE", "")]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(empty_environment.api_base, DEFAULT_API_BASE);
}

#[test]
fn block_on_error_takes_the_argument_then_the_environment() {
    let explicit_false = PromptGuardGuardrail::new(
        args(Some("key"), Some(false)),
        &env(&[("PROMPTGUARD_BLOCK_ON_ERROR", "true")]),
        client(),
    )
    .expect("guardrail");
    assert!(!explicit_false.block_on_error);
    let explicit_true = PromptGuardGuardrail::new(
        args(Some("key"), Some(true)),
        &env(&[("PROMPTGUARD_BLOCK_ON_ERROR", "false")]),
        client(),
    )
    .expect("guardrail");
    assert!(explicit_true.block_on_error);
    let default =
        PromptGuardGuardrail::new(args(Some("key"), None), &env(&[]), client()).expect("guardrail");
    assert!(default.block_on_error);
    let empty_environment = PromptGuardGuardrail::new(
        args(Some("key"), None),
        &env(&[("PROMPTGUARD_BLOCK_ON_ERROR", "  ")]),
        client(),
    )
    .expect("guardrail");
    assert!(empty_environment.block_on_error);
}

#[rstest]
#[case::true_text("true", true)]
#[case::one("1", true)]
#[case::yes("YES", true)]
#[case::on("On", true)]
#[case::false_text("false", false)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::off("OFF", false)]
fn block_on_error_reads_the_environment_flag(#[case] text: &str, #[case] expected: bool) {
    let guardrail = PromptGuardGuardrail::new(
        args(Some("key"), None),
        &env(&[("PROMPTGUARD_BLOCK_ON_ERROR", text)]),
        client(),
    )
    .expect("guardrail");
    assert_eq!(guardrail.block_on_error, expected);
}

#[test]
fn block_on_error_environment_text_that_is_not_a_bool_names_the_variable() {
    let error = PromptGuardGuardrail::new(
        args(Some("key"), None),
        &env(&[("PROMPTGUARD_BLOCK_ON_ERROR", "maybe")]),
        client(),
    )
    .unwrap_err();
    assert!(matches!(error, PromptGuardError::Environment { .. }));
    assert!(error.to_string().contains("PROMPTGUARD_BLOCK_ON_ERROR"));
}

#[test]
fn timeout_defaults_to_ten_seconds_and_takes_the_configured_value() {
    let by_default =
        PromptGuardGuardrail::new(args(Some("key"), None), &env(&[]), client()).expect("guardrail");
    assert_eq!(by_default.timeout, Duration::from_secs(10));
    let mut request = args(Some("key"), None);
    request.timeout = Some(2.5);
    let configured = PromptGuardGuardrail::new(request, &env(&[]), client()).expect("guardrail");
    assert_eq!(configured.timeout, Duration::from_millis(2500));
}

#[test]
fn a_timeout_that_is_not_a_duration_is_refused() {
    let mut request = args(Some("key"), None);
    request.timeout = Some(-1.0);
    let error = PromptGuardGuardrail::new(request, &env(&[]), client()).unwrap_err();
    assert!(matches!(
        error,
        PromptGuardError::UnrepresentableTimeout { .. }
    ));
}

#[test]
fn request_headers_carry_the_api_key_and_the_json_content_type() {
    let guardrail = PromptGuardGuardrail::new(
        with_base(args(Some("test-key"), None), Some("https://guard.test")),
        &env(&[]),
        client(),
    )
    .expect("guardrail");
    let headers = guardrail.request_headers().expect("headers");
    let api_key = headers.get("x-api-key").expect("api key");
    assert_eq!(api_key.to_str().expect("text"), "test-key");
    assert!(api_key.is_sensitive());
    let content_type = headers.get("content-type").expect("content type");
    assert_eq!(content_type.to_str().expect("text"), "application/json");
    assert!(!content_type.is_sensitive());
}

#[test]
fn texts_become_user_messages_for_a_request() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["first".to_owned(), "second".to_owned()]);
    inputs.model = Some("gpt-4o".to_owned());
    let request = built(inputs, GuardrailInputType::Request).expect("request");
    assert_eq!(request.direction, Direction::Input);
    assert_eq!(request.model.as_deref(), Some("gpt-4o"));
    assert_eq!(request.images, None);
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[0].role, "user");
    assert_eq!(
        request.messages[0].extra.get("content"),
        Some(&GuardrailValue::Text("first".to_owned()))
    );
    assert_eq!(request.messages[1].role, "user");
    assert_eq!(
        request.messages[1].extra.get("content"),
        Some(&GuardrailValue::Text("second".to_owned()))
    );
}

#[test]
fn a_response_scan_uses_the_output_direction() {
    let request = built(text_inputs(), GuardrailInputType::Response).expect("request");
    assert_eq!(request.direction, Direction::Output);
    assert_eq!(
        serde_json::to_string(&request.direction).expect("direction"),
        "\"output\""
    );
}

#[test]
fn structured_messages_win_over_texts() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["text".to_owned()]);
    inputs.structured_messages = Some(vec![GuardrailMessage {
        role: "system".to_owned(),
        extra: BTreeMap::new(),
    }]);
    let request = built(inputs, GuardrailInputType::Request).expect("request");
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, "system");
}

#[test]
fn empty_structured_messages_fall_back_to_the_texts() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.structured_messages = Some(Vec::new());
    inputs.texts = Some(vec!["text".to_owned()]);
    let request = built(inputs, GuardrailInputType::Request).expect("request");
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, "user");
}

#[rstest]
#[case::nothing(None)]
#[case::no_texts(Some(Vec::new()))]
fn no_messages_makes_no_request(#[case] texts: Option<Vec<String>>) {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = texts;
    assert!(built(inputs, GuardrailInputType::Request).is_none());
}

#[test]
fn images_are_sent_only_when_they_are_not_empty() {
    let mut inputs = text_inputs();
    inputs.images = Some(Vec::new());
    let request = built(inputs, GuardrailInputType::Request).expect("request");
    assert_eq!(request.images, None);
    let mut inputs = text_inputs();
    inputs.images = Some(vec!["https://img.test/a.png".to_owned()]);
    let request = built(inputs, GuardrailInputType::Request).expect("request");
    assert_eq!(
        request.images,
        Some(vec!["https://img.test/a.png".to_owned()])
    );
}

#[rstest]
#[case::allow(Some("allow"))]
#[case::other(Some("monitor"))]
#[case::absent(None)]
fn any_other_decision_allows_the_inputs(#[case] decision: Option<&str>) {
    let inputs = text_inputs();
    let response = GuardResponse {
        decision: decision.map(ToOwned::to_owned),
        threat_type: Some("topic".to_owned()),
        event_id: Some("evt".to_owned()),
        confidence: Some(0.5),
        redacted_messages: None,
    };
    let updated = wire::apply_decision(inputs.clone(), response).expect("allowed");
    assert_eq!(updated, inputs);
}

#[test]
fn a_block_decision_raises_the_documented_message() {
    let response = GuardResponse {
        decision: Some("block".to_owned()),
        threat_type: Some("prompt_injection".to_owned()),
        event_id: Some("evt-1".to_owned()),
        confidence: Some(0.9),
        redacted_messages: None,
    };
    let error = wire::apply_decision(text_inputs(), response).unwrap_err();
    assert!(error.raised().blocked_content);
    assert_eq!(error.raised().status, 400);
    assert_eq!(
        error.to_string(),
        "Blocked by PromptGuard: prompt_injection (confidence=0.9, event_id=evt-1)"
    );
}

#[test]
fn a_block_without_details_uses_the_defaults() {
    let response = GuardResponse {
        decision: Some("block".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: None,
    };
    let error = wire::apply_decision(text_inputs(), response).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Blocked by PromptGuard: unknown (confidence=0.0, event_id=)"
    );
}

fn message(role: &str, content: GuardrailValue) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([("content".to_owned(), content)]),
    }
}

fn part(kind: &str, text: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("type".to_owned(), GuardrailValue::Text(kind.to_owned())),
        ("text".to_owned(), GuardrailValue::Text(text.to_owned())),
    ]))
}

fn redacted_messages() -> Vec<GuardrailMessage> {
    vec![
        message("user", GuardrailValue::Text("hello <EMAIL>".to_owned())),
        message(
            "assistant",
            GuardrailValue::Text("assistant <EMAIL>".to_owned()),
        ),
        message(
            "user",
            GuardrailValue::Array(vec![
                part("text", "call <PHONE>"),
                part("image_url", "ignored"),
                part("text", ""),
            ]),
        ),
    ]
}

#[test]
fn a_redact_decision_rewrites_messages_and_user_texts() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["original".to_owned()]);
    inputs.structured_messages = Some(vec![GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text("hello".to_owned()),
        )]),
    }]);
    let redacted = redacted_messages();
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: Some(redacted.clone()),
    };
    let updated = wire::apply_decision(inputs, response).expect("redacted");
    assert_eq!(updated.structured_messages, Some(redacted));
    assert_eq!(
        updated.texts,
        Some(vec!["hello <EMAIL>".to_owned(), "call <PHONE>".to_owned()])
    );
}

#[test]
fn a_redact_decision_leaves_texts_alone_when_the_input_has_none() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.structured_messages = Some(vec![GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text("hello".to_owned()),
        )]),
    }]);
    let redacted = redacted_messages();
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: Some(redacted.clone()),
    };
    let updated = wire::apply_decision(inputs, response).expect("redacted");
    assert_eq!(updated.structured_messages, Some(redacted));
    assert_eq!(updated.texts, None);
}

#[test]
fn a_redact_decision_without_structured_messages_only_touches_texts() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["original".to_owned()]);
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: Some(vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([(
                "content".to_owned(),
                GuardrailValue::Text("masked".to_owned()),
            )]),
        }]),
    };
    let updated = wire::apply_decision(inputs, response).expect("redacted");
    assert_eq!(updated.structured_messages, None);
    assert_eq!(updated.texts, Some(vec!["masked".to_owned()]));
}

#[test]
fn a_redact_decision_without_user_texts_keeps_the_existing_texts() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["original".to_owned()]);
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: Some(vec![
            GuardrailMessage {
                role: "assistant".to_owned(),
                extra: BTreeMap::from([(
                    "content".to_owned(),
                    GuardrailValue::Text("assistant".to_owned()),
                )]),
            },
            GuardrailMessage {
                role: "user".to_owned(),
                extra: BTreeMap::from([("content".to_owned(), GuardrailValue::Null)]),
            },
        ]),
    };
    let updated = wire::apply_decision(inputs, response).expect("redacted");
    assert_eq!(updated.texts, Some(vec!["original".to_owned()]));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(Vec::new()))]
fn a_redact_decision_without_redacted_messages_changes_nothing(
    #[case] redacted: Option<Vec<GuardrailMessage>>,
) {
    let inputs = text_inputs();
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: redacted,
    };
    let updated = wire::apply_decision(inputs.clone(), response).expect("redacted");
    assert_eq!(updated, inputs);
}

#[test]
fn an_empty_string_user_content_still_counts_as_extracted_text() {
    let mut inputs = GuardrailApiInputs::default();
    inputs.texts = Some(vec!["original".to_owned()]);
    let response = GuardResponse {
        decision: Some("redact".to_owned()),
        threat_type: None,
        event_id: None,
        confidence: None,
        redacted_messages: Some(vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([("content".to_owned(), GuardrailValue::Text(String::new()))]),
        }]),
    };
    let updated = wire::apply_decision(inputs, response).expect("redacted");
    assert_eq!(updated.texts, Some(vec![String::new()]));
}

#[test]
fn no_messages_passes_the_inputs_through_without_a_call() {
    let inputs = GuardrailApiInputs::default();
    let guardrail = PromptGuardGuardrail::new(
        with_base(args(Some("key"), Some(true)), Some("http://127.0.0.1:9")),
        &env(&[]),
        client(),
    )
    .expect("guardrail");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let updated = runtime
        .block_on(guardrail.apply_guardrail(inputs.clone(), GuardrailInputType::Request))
        .expect("passthrough");
    assert_eq!(updated, inputs);
}

#[derive(Debug, Deserialize)]
struct RecordedBody {
    messages: Vec<GuardrailMessage>,
    direction: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    images: Option<Vec<String>>,
}

#[derive(Debug)]
struct Recorded {
    method: String,
    path: String,
    api_key: Option<String>,
    content_type: Option<String>,
    body: RecordedBody,
}

type Captured = Arc<Mutex<Option<Recorded>>>;

#[derive(Serialize)]
struct GuardReply {
    decision: String,
}

async fn record(
    State(captured): State<Captured>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<RecordedBody>,
) -> Json<GuardReply> {
    let recorded = Recorded {
        method: method.to_string(),
        path: uri.path().to_owned(),
        api_key: header(&headers, "x-api-key"),
        content_type: header(&headers, "content-type"),
        body,
    };
    *captured.lock().expect("capture") = Some(recorded);
    Json(GuardReply {
        decision: "allow".to_owned(),
    })
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

async fn serve(router: Router) -> (String, JoinSet<std::io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

fn guardrail_at(base: &str, block_on_error: bool) -> PromptGuardGuardrail {
    let request = with_base(args(Some("test-key"), Some(block_on_error)), Some(base));
    PromptGuardGuardrail::new(request, &env(&[]), client()).expect("guardrail")
}

#[tokio::test]
async fn the_request_reaches_the_guard_endpoint_with_the_key_and_payload() {
    let captured: Captured = Arc::new(Mutex::new(None));
    let router = Router::new()
        .route("/api/v1/guard", post(record))
        .with_state(Arc::clone(&captured));
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let mut inputs = text_inputs();
    inputs.model = Some("gpt-4o".to_owned());
    let updated = guardrail
        .apply_guardrail(inputs.clone(), GuardrailInputType::Request)
        .await
        .expect("allowed");
    assert_eq!(updated, inputs);
    let recorded = captured.lock().expect("capture").take().expect("a request");
    assert_eq!(recorded.method, "POST");
    assert_eq!(recorded.path, "/api/v1/guard");
    assert_eq!(recorded.api_key.as_deref(), Some("test-key"));
    assert_eq!(recorded.content_type.as_deref(), Some("application/json"));
    assert_eq!(recorded.body.direction, "input");
    assert_eq!(recorded.body.model.as_deref(), Some("gpt-4o"));
    assert_eq!(recorded.body.images, None);
    assert_eq!(recorded.body.messages.len(), 1);
    assert_eq!(recorded.body.messages[0].role, "user");
    server.shutdown().await;
}

#[tokio::test]
async fn a_response_scan_sends_the_output_direction_and_the_images() {
    let captured: Captured = Arc::new(Mutex::new(None));
    let router = Router::new()
        .route("/api/v1/guard", post(record))
        .with_state(Arc::clone(&captured));
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let mut inputs = GuardrailApiInputs::default();
    inputs.structured_messages = Some(vec![GuardrailMessage {
        role: "assistant".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text("answer".to_owned()),
        )]),
    }]);
    inputs.images = Some(vec!["https://img.test/a.png".to_owned()]);
    let updated = guardrail
        .apply_guardrail(inputs.clone(), GuardrailInputType::Response)
        .await
        .expect("allowed");
    assert_eq!(updated, inputs);
    let recorded = captured.lock().expect("capture").take().expect("a request");
    assert_eq!(recorded.body.direction, "output");
    assert_eq!(
        recorded.body.images,
        Some(vec!["https://img.test/a.png".to_owned()])
    );
    assert_eq!(recorded.body.messages.len(), 1);
    assert_eq!(recorded.body.messages[0].role, "assistant");
    server.shutdown().await;
}

#[tokio::test]
async fn a_failed_call_raises_when_block_on_error_is_true() {
    let router = Router::new().route(
        "/api/v1/guard",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let error = guardrail
        .apply_guardrail(text_inputs(), GuardrailInputType::Request)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptGuardError::HttpStatus { status } if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert!(
        error
            .to_string()
            .starts_with("PromptGuard API unreachable (block_on_error=True): ")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_failed_call_returns_the_inputs_when_block_on_error_is_false() {
    let router = Router::new().route(
        "/api/v1/guard",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, false);
    let inputs = text_inputs();
    let updated = guardrail
        .apply_guardrail(inputs.clone(), GuardrailInputType::Request)
        .await
        .expect("fail open");
    assert_eq!(updated, inputs);
    server.shutdown().await;
}

#[tokio::test]
async fn an_answer_that_is_not_json_raises() {
    let router = Router::new().route("/api/v1/guard", post(|| async { "not json" }));
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let error = guardrail
        .apply_guardrail(text_inputs(), GuardrailInputType::Request)
        .await
        .unwrap_err();
    assert!(matches!(error, PromptGuardError::ResponseJson { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn an_answer_larger_than_the_limit_is_refused() {
    let oversized = "x".repeat(MAX_RESPONSE_BYTES.checked_add(1).expect("size"));
    let router = Router::new().route("/api/v1/guard", post(move || async move { oversized }));
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let error = guardrail
        .apply_guardrail(text_inputs(), GuardrailInputType::Request)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptGuardError::ResponseTooLarge { limit } if limit == MAX_RESPONSE_BYTES
    ));
    server.shutdown().await;
}

#[tokio::test]
async fn an_answer_of_exactly_the_limit_is_read() {
    let target = MAX_RESPONSE_BYTES.checked_sub(2).expect("target");
    let mut body = String::from(r#"{"decision":"allow","threat_type":""#);
    while body.len() < target {
        body.push('x');
    }
    body.push_str(r#""}"#);
    assert_eq!(body.len(), MAX_RESPONSE_BYTES);
    let router = Router::new().route("/api/v1/guard", post(move || async move { body }));
    let (base, mut server) = serve(router).await;
    let guardrail = guardrail_at(&base, true);
    let inputs = text_inputs();
    let updated = guardrail
        .apply_guardrail(inputs.clone(), GuardrailInputType::Request)
        .await
        .expect("allowed");
    assert_eq!(updated, inputs);
    server.shutdown().await;
}
