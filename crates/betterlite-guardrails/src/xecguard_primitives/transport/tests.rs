#![cfg(test)]

use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::{call_grounding, call_scan};
use crate::xecguard_primitives::error::XecGuardError;
use crate::xecguard_primitives::settings::XecGuardSettings;
use crate::xecguard_primitives::wire::{GroundingDocument, ScanType, XecGuardMessage};

type Server = JoinSet<Result<(), io::Error>>;

#[derive(Debug, Default, Clone)]
struct Captured {
    requests: usize,
    path: String,
    authorization: String,
    content_type: String,
    body: Vec<u8>,
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn serve(
    status: StatusCode,
    response_body: String,
) -> (String, Arc<Mutex<Captured>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let captured = Arc::new(Mutex::new(Captured::default()));
    let handler_captured = Arc::clone(&captured);
    let handler = move |headers: HeaderMap, uri: Uri, body: Bytes| {
        let request_capture = Arc::clone(&handler_captured);
        let answer = response_body.clone();
        async move {
            let mut request = request_capture.lock().expect("captured request");
            request.requests = request.requests.saturating_add(1);
            request.path = uri.path().to_owned();
            request.authorization = header(&headers, AUTHORIZATION.as_str());
            request.content_type = header(&headers, CONTENT_TYPE.as_str());
            request.body = body.to_vec();
            drop(request);
            (status, answer)
        }
    };
    let router = Router::new()
        .route("/xecguard/v1/scan", post(handler.clone()))
        .route("/xecguard/v1/grounding", post(handler));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

fn settings(api_base: &str) -> XecGuardSettings {
    XecGuardSettings {
        api_key: SecretString::from("secret-key"),
        api_base: api_base.to_owned(),
        model: "xecguard_v2".to_owned(),
        policy_names: None,
        block_on_error: true,
        grounding_strictness: "BALANCED".to_owned(),
        timeout: None,
    }
}

fn message(role: &str, content: &str) -> XecGuardMessage {
    XecGuardMessage {
        role: role.to_owned(),
        content: content.to_owned(),
    }
}

#[derive(Debug, Deserialize)]
struct CapturedScan {
    model: String,
    scan_type: String,
    messages: Vec<XecGuardMessage>,
    policy_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CapturedGrounding {
    model: String,
    prompt: String,
    response: String,
    documents: Vec<GroundingDocument>,
    strictness: String,
}

#[tokio::test]
async fn transport_scan_posts_messages_with_the_default_policies() {
    let (base, captured, _server) = serve(
        StatusCode::OK,
        r#"{"decision":"SAFE","trace_id":"t-1"}"#.to_owned(),
    )
    .await;
    let messages = [message("system", "policy"), message("user", "hello")];
    let result = call_scan(
        &reqwest::Client::default(),
        &settings(&base),
        &messages,
        ScanType::Input,
        false,
    )
    .await
    .expect("scan")
    .expect("result");
    assert_eq!(result.trace_id_text(), "t-1");
    assert!(!result.is_unsafe());
    let observed = captured.lock().expect("captured request").clone();
    assert_eq!(observed.requests, 1);
    assert_eq!(observed.path, "/xecguard/v1/scan");
    assert_eq!(observed.authorization, "Bearer secret-key");
    assert_eq!(observed.content_type, "application/json");
    let payload: CapturedScan = serde_json::from_slice(&observed.body).expect("scan payload");
    assert_eq!(payload.model, "xecguard_v2");
    assert_eq!(payload.scan_type, "input");
    assert_eq!(payload.messages, messages);
    assert_eq!(
        payload.policy_names,
        [
            "Default_Policy_SystemPromptEnforcement",
            "Default_Policy_HarmfulContentProtection",
            "Default_Policy_GeneralPromptAttackProtection",
        ]
    );
}

#[rstest]
#[case::configured(vec!["Custom".to_owned()], vec!["Custom".to_owned()])]
#[case::explicit_empty(Vec::new(), Vec::new())]
#[tokio::test]
async fn transport_scan_uses_the_configured_policy_names(
    #[case] configured: Vec<String>,
    #[case] expected: Vec<String>,
) {
    let (base, captured, _server) = serve(StatusCode::OK, "{}".to_owned()).await;
    let mut settings = settings(&base);
    settings.policy_names = Some(configured);
    call_scan(
        &reqwest::Client::default(),
        &settings,
        &[message("user", "hi")],
        ScanType::Response,
        false,
    )
    .await
    .expect("scan");
    let observed = captured.lock().expect("captured request").clone();
    let payload: CapturedScan = serde_json::from_slice(&observed.body).expect("scan payload");
    assert_eq!(payload.policy_names, expected);
    assert_eq!(payload.scan_type, "response");
}

#[tokio::test]
async fn transport_scan_returns_none_when_errors_are_suppressed() {
    let (base, captured, _server) =
        serve(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    let result = call_scan(
        &reqwest::Client::default(),
        &settings(&base),
        &[message("user", "hi")],
        ScanType::Input,
        true,
    )
    .await;
    assert!(result.expect("suppressed failure").is_none());
    assert_eq!(captured.lock().expect("captured request").requests, 1);
}

#[tokio::test]
async fn transport_scan_returns_none_when_block_on_error_is_off() {
    let (base, _captured, _server) =
        serve(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    let mut settings = settings(&base);
    settings.block_on_error = false;
    let result = call_scan(
        &reqwest::Client::default(),
        &settings,
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await;
    assert!(result.expect("unblocked failure").is_none());
}

#[tokio::test]
async fn transport_scan_reports_an_unreachable_api_when_block_on_error() {
    let (base, _captured, _server) =
        serve(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    let error = call_scan(
        &reqwest::Client::default(),
        &settings(&base),
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await
    .expect_err("blocked on error");
    assert!(matches!(error, XecGuardError::Unreachable { .. }));
    let rendered = error.to_string();
    assert!(rendered.starts_with("XecGuard API unreachable (block_on_error=True): "));
    assert!(rendered.contains("500"));
}

#[tokio::test]
async fn transport_scan_rejects_a_response_that_is_not_a_json_object() {
    let (base, _captured, _server) = serve(StatusCode::OK, "[]".to_owned()).await;
    let error = call_scan(
        &reqwest::Client::default(),
        &settings(&base),
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await
    .expect_err("non-object response");
    assert!(error.to_string().contains("not valid JSON"));
}

#[tokio::test]
async fn transport_scan_rejects_a_response_over_one_mebibyte() {
    let (base, _captured, _server) = serve(StatusCode::OK, " ".repeat(1024 * 1024 + 1)).await;
    let error = call_scan(
        &reqwest::Client::default(),
        &settings(&base),
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await
    .expect_err("oversized response");
    assert!(error.to_string().contains("larger than 1048576 bytes"));
}

#[tokio::test]
async fn transport_reports_an_api_key_that_cannot_be_a_header() {
    let mut settings = settings("http://127.0.0.1:1");
    settings.api_key = SecretString::from("\u{0}");
    let error = call_scan(
        &reqwest::Client::default(),
        &settings,
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await
    .expect_err("invalid header");
    assert!(error.to_string().contains("not a valid header"));
}

#[tokio::test]
async fn transport_reports_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let answer = || async {
        sleep(Duration::from_millis(200)).await;
        (StatusCode::OK, "{}".to_owned())
    };
    let router = Router::new().route("/xecguard/v1/scan", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let mut settings = settings(&base);
    settings.timeout = Some(Duration::from_millis(1));
    let error = call_scan(
        &reqwest::Client::default(),
        &settings,
        &[message("user", "hi")],
        ScanType::Input,
        false,
    )
    .await
    .expect_err("timed out");
    assert!(error.to_string().contains("timed out"));
}

#[tokio::test]
async fn transport_grounding_posts_the_last_user_and_assistant_texts() {
    let (base, captured, _server) =
        serve(StatusCode::OK, r#"{"decision":"SAFE"}"#.to_owned()).await;
    let messages = [
        message("system", "policy"),
        message("user", "old"),
        message("user", "latest"),
        message("assistant", "answer"),
    ];
    let documents = [GroundingDocument {
        document_id: "d1".to_owned(),
        context: "ctx".to_owned(),
    }];
    let mut settings = settings(&base);
    settings.api_base = format!("{base}/");
    let result = call_grounding(
        &reqwest::Client::default(),
        &settings,
        &messages,
        &documents,
    )
    .await
    .expect("grounding")
    .expect("result");
    assert!(!result.is_unsafe());
    let observed = captured.lock().expect("captured request").clone();
    assert_eq!(observed.path, "/xecguard/v1/grounding");
    let payload: CapturedGrounding = serde_json::from_slice(&observed.body).expect("payload");
    assert_eq!(payload.model, "xecguard_v2");
    assert_eq!(payload.prompt, "latest");
    assert_eq!(payload.response, "answer");
    assert_eq!(payload.documents, documents);
    assert_eq!(payload.strictness, "BALANCED");
}

#[rstest]
#[case::no_assistant(vec![message("user", "question")])]
#[case::empty_assistant(vec![message("user", "question"), message("assistant", "")])]
#[case::no_user(vec![message("assistant", "answer")])]
#[tokio::test]
async fn transport_grounding_returns_none_without_both_texts(
    #[case] messages: Vec<XecGuardMessage>,
) {
    let (base, captured, _server) = serve(StatusCode::OK, "{}".to_owned()).await;
    let result = call_grounding(
        &reqwest::Client::default(),
        &settings(&base),
        &messages,
        &[],
    )
    .await
    .expect("skipped grounding");
    assert!(result.is_none());
    assert_eq!(captured.lock().expect("captured request").requests, 0);
}
