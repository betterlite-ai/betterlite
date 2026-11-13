use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::future::IntoFuture as _;
use std::process;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::set_default;

use super::{array_of, body, field_text, list, object, text};
use crate::cato::error::CatoError;
use crate::cato::{
    CallHook, CatoGuardrail, CatoParams, DEFAULT_BLOCK_MESSAGE, DEFAULT_TIMEOUT, LITELLM_VERSION,
    MAX_RESPONSE_BYTES,
};

type Captured = Arc<Mutex<Vec<(BTreeMap<String, String>, String)>>>;
type Server = JoinSet<Result<(), std::io::Error>>;

#[derive(Debug, Clone, Serialize)]
struct Action {
    action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detection_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Analysis {
    policy_drill_down: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Serialize)]
struct Answer {
    #[serde(skip_serializing_if = "Option::is_none")]
    required_action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analysis_result: Option<Analysis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redacted_chat: Option<BTreeMap<String, GuardrailValue>>,
}

fn no_action() -> Answer {
    Answer {
        required_action: None,
        analysis_result: None,
        redacted_chat: None,
    }
}

fn action(action_type: &str, detection_message: Option<&str>) -> Answer {
    Answer {
        required_action: Some(Action {
            action_type: action_type.to_owned(),
            detection_message: detection_message.map(str::to_owned),
        }),
        analysis_result: None,
        redacted_chat: None,
    }
}

async fn serve(status: StatusCode, answer: Answer) -> (String, Server, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .route("/fw/v1/analyze", post(handle))
        .with_state((Arc::clone(&captured), status, answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server, captured)
}

async fn serve_raw(raw: String) -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/fw/v1/analyze",
        post(move || {
            let raw = raw.clone();
            async move { ([(CONTENT_TYPE, "application/json")], raw) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

fn raw_answer_of_size(size: usize) -> String {
    format!("{{\"padding\":\"{}\"}}", "a".repeat(size - 14))
}

async fn handle(
    State((captured, status, answer)): State<(Captured, StatusCode, Answer)>,
    headers: HeaderMap,
    request_body: String,
) -> (StatusCode, Json<Answer>) {
    let mut recorded = BTreeMap::new();
    for (name, value) in &headers {
        if let Ok(value) = value.to_str() {
            recorded.insert(name.as_str().to_owned(), value.to_owned());
        }
    }
    captured
        .lock()
        .expect("captured")
        .push((recorded, request_body));
    (status, Json(answer))
}

#[derive(Debug, Deserialize)]
struct PostedBody {
    messages: Vec<GuardrailValue>,
}

fn build(base: &str) -> CatoGuardrail {
    CatoGuardrail::new(
        CatoParams {
            guardrail_name: Some("cato".to_owned()),
            api_key: Some(SecretString::from("test-key")),
            api_base: Some(base.to_owned()),
            inspect_embeddings: None,
            ssl_verify: None,
            timeout: None,
        },
        &BTreeMap::<String, String>::new(),
    )
    .expect("guardrail")
}

fn chat_data() -> BTreeMap<String, GuardrailValue> {
    body(&[
        ("litellm_call_id", text("call-1")),
        (
            "messages",
            list(&[object(&[
                ("content", text("hello")),
                ("role", text("user")),
            ])]),
        ),
    ])
}

#[tokio::test]
async fn cato_guardrail_call_posts_the_analyze_messages_and_headers() {
    let (base, mut server, captured) = serve(StatusCode::OK, no_action()).await;
    let mut data = chat_data();
    build(&base)
        .call(
            &mut data,
            CallHook::PreCall,
            Some("alias"),
            Some("user@example.test"),
        )
        .await
        .expect("call");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    assert_eq!(recorded.len(), 1);
    let (headers, body) = recorded.first().expect("request");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer test-key")
    );
    assert_eq!(
        headers.get("x-cato-litellm-hook").map(String::as_str),
        Some("pre_call")
    );
    assert_eq!(
        headers.get("x-cato-litellm-version").map(String::as_str),
        Some(LITELLM_VERSION)
    );
    assert_eq!(
        headers.get("x-cato-call-id").map(String::as_str),
        Some("call-1")
    );
    assert_eq!(
        headers.get("x-cato-user-email").map(String::as_str),
        Some("user@example.test")
    );
    assert_eq!(
        headers.get("x-cato-gateway-key-alias").map(String::as_str),
        Some("alias")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("user-agent").map(String::as_str),
        Some(format!("litellm/{LITELLM_VERSION}").as_str())
    );
    let posted: PostedBody = serde_json::from_str(body).expect("posted body");
    assert_eq!(
        posted.messages,
        vec![object(&[
            ("content", text("hello")),
            ("role", text("user"))
        ])]
    );
}

#[tokio::test]
async fn cato_guardrail_headers_omit_absent_optional_values() {
    let (base, mut server, captured) = serve(StatusCode::OK, no_action()).await;
    let mut data = body(&[("messages", list(&[object(&[("content", text("hi"))])]))]);
    build(&base)
        .call(&mut data, CallHook::Moderation, None, None)
        .await
        .expect("call");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    let (headers, _body) = recorded.first().expect("request");
    assert_eq!(
        headers.get("x-cato-litellm-hook").map(String::as_str),
        Some("moderation")
    );
    assert!(!headers.contains_key("x-cato-call-id"));
    assert!(!headers.contains_key("x-cato-user-email"));
    assert!(!headers.contains_key("x-cato-gateway-key-alias"));
}

#[tokio::test]
async fn cato_guardrail_user_email_header_ignores_a_caller_supplied_end_user_id() {
    let (base, mut server, captured) = serve(StatusCode::OK, no_action()).await;
    let mut data = body(&[
        ("end_user_id", text("spoofed@example.test")),
        ("user", text("spoofed@example.test")),
        ("messages", list(&[object(&[("content", text("hi"))])])),
    ]);
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("call");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    let (headers, _body) = recorded.first().expect("request");
    assert!(!headers.contains_key("x-cato-user-email"));
}

#[tokio::test]
async fn cato_guardrail_headers_omit_a_call_id_that_is_not_text() {
    let (base, mut server, captured) = serve(StatusCode::OK, no_action()).await;
    let mut data = body(&[
        ("litellm_call_id", GuardrailValue::SignedInteger(5)),
        ("messages", list(&[object(&[("content", text("hi"))])])),
    ]);
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("call");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    let (headers, _body) = recorded.first().expect("request");
    assert!(!headers.contains_key("x-cato-call-id"));
}

#[tokio::test]
async fn cato_guardrail_headers_reject_a_value_that_is_not_a_header() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let mut configured = build(&base);
    configured.timeout = Duration::from_millis(100);
    let mut data = body(&[("messages", list(&[object(&[("content", text("hi"))])]))]);
    let error = configured
        .call(&mut data, CallHook::PreCall, None, Some("bad\nvalue"))
        .await
        .expect_err("invalid header");
    assert!(matches!(error, CatoError::InvalidHeader { .. }));
}

#[tokio::test]
async fn cato_guardrail_missing_action_returns_the_request_unchanged() {
    let (base, mut server, _captured) = serve(StatusCode::OK, no_action()).await;
    let mut data = chat_data();
    let original = data.clone();
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("pass through");
    server.shutdown().await;
    assert_eq!(data, original);
}

#[tokio::test]
async fn cato_guardrail_monitor_action_returns_the_request_unchanged() {
    let (base, mut server, _captured) = serve(StatusCode::OK, action("monitor_action", None)).await;
    let mut data = chat_data();
    let original = data.clone();
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("pass through");
    server.shutdown().await;
    assert_eq!(data, original);
}

#[tokio::test]
async fn cato_guardrail_block_action_returns_a_400_error_with_the_detection_message() {
    let log_path = env::temp_dir().join(format!("betterlite-cato-block-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let mut answer = action("block_action", Some("blocked by the finance policy"));
    answer.analysis_result = Some(Analysis {
        policy_drill_down: BTreeMap::from([(
            "finance-policy".to_owned(),
            GuardrailValue::Object(BTreeMap::new()),
        )]),
    });
    let (base, mut server, _captured) = serve(StatusCode::OK, answer).await;
    let guard = set_default(subscriber);
    let mut data = chat_data();
    let error = build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect_err("blocked");
    drop(guard);
    server.shutdown().await;
    assert!(matches!(
        &error,
        CatoError::Blocked { message } if message == "blocked by the finance policy"
    ));
    assert_eq!(error.status_code().as_u16(), 400);
    assert_eq!(
        crate::cato::error::INVALID_REQUEST_ERROR,
        "invalid_request_error"
    );
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("finance-policy"));
    assert!(!written.contains("test-key"));
}

#[tokio::test]
async fn cato_guardrail_block_action_without_a_detection_message_uses_its_own_wording() {
    let (base, mut server, _captured) = serve(StatusCode::OK, action("block_action", None)).await;
    let mut data = chat_data();
    let error = build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect_err("blocked");
    server.shutdown().await;
    assert!(matches!(
        &error,
        CatoError::Blocked { message } if message == DEFAULT_BLOCK_MESSAGE
    ));
}

#[tokio::test]
async fn cato_guardrail_unknown_action_logs_an_error_and_returns_the_request_unchanged() {
    let log_path = env::temp_dir().join(format!("betterlite-cato-unknown-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let (base, mut server, _captured) =
        serve(StatusCode::OK, action("surprise_action", None)).await;
    let guard = set_default(subscriber);
    let mut data = chat_data();
    let original = data.clone();
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("pass through");
    drop(guard);
    server.shutdown().await;
    assert_eq!(data, original);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("ERROR"));
    assert!(written.contains("surprise_action"));
}

#[tokio::test]
async fn cato_guardrail_anonymize_action_rewrites_the_inspected_request() {
    let mut answer = action("anonymize_action", None);
    answer.redacted_chat = Some(BTreeMap::from([(
        "all_redacted_messages".to_owned(),
        list(&[object(&[
            ("content", text("REDACTED")),
            ("role", text("user")),
        ])]),
    )]));
    let (base, mut server, _captured) = serve(StatusCode::OK, answer).await;
    let mut data = body(&[(
        "messages",
        list(&[object(&[
            ("content", text("secret")),
            ("role", text("user")),
        ])]),
    )]);
    build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect("rewritten");
    server.shutdown().await;
    assert_eq!(
        field_text(&array_of(&data, "messages"), "content"),
        ["REDACTED"]
    );
}

#[tokio::test]
async fn cato_guardrail_returns_the_upstream_error_status() {
    let (base, mut server, _captured) = serve(StatusCode::INTERNAL_SERVER_ERROR, no_action()).await;
    let mut data = chat_data();
    let error = build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect_err("upstream error");
    server.shutdown().await;
    assert!(matches!(
        error,
        CatoError::HttpStatus { status } if status.as_u16() == 500
    ));
}

#[tokio::test]
async fn cato_guardrail_uses_the_default_timeout_when_none_is_configured() {
    let (base, mut server, _captured) = serve(StatusCode::OK, no_action()).await;
    let guardrail = build(&base);
    server.shutdown().await;
    assert_eq!(guardrail.timeout, DEFAULT_TIMEOUT);
}

#[tokio::test]
async fn cato_guardrail_accepts_an_answer_of_exactly_the_size_limit() {
    let (base, mut server) = serve_raw(raw_answer_of_size(MAX_RESPONSE_BYTES)).await;
    let mut data = chat_data();
    let result = build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await;
    server.shutdown().await;
    result.expect("an answer at the limit is read");
}

#[tokio::test]
async fn cato_guardrail_rejects_an_answer_over_the_size_limit() {
    let (base, mut server) = serve_raw(raw_answer_of_size(MAX_RESPONSE_BYTES + 1)).await;
    let mut data = chat_data();
    let error = build(&base)
        .call(&mut data, CallHook::PreCall, None, None)
        .await
        .expect_err("too large");
    server.shutdown().await;
    assert!(matches!(
        error,
        CatoError::ResponseTooLarge { limit } if limit == MAX_RESPONSE_BYTES
    ));
}
