use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode as AxumStatus;
use axum::routing::post;
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{fixed_clock, guardrail, guardrail_at, inputs, log_path, log_to};
use crate::akto::AktoMirroring;
use crate::akto::error::AktoError;
use crate::akto::payload::AktoPayload;
use crate::akto::transport::AktoAnswer;

struct Recorded {
    path: String,
    query: Option<String>,
    content_type: Option<String>,
    authorization: Option<String>,
    body: Vec<u8>,
}

type Captures = Arc<Mutex<Vec<Recorded>>>;

fn header_text(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn capture(State(captures): State<Captures>, request: Request) -> (AxumStatus, &'static str) {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .expect("body bytes");
    captures.lock().expect("captures").push(Recorded {
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(str::to_owned),
        content_type: header_text(&parts.headers, "content-type"),
        authorization: header_text(&parts.headers, "authorization"),
        body: bytes.to_vec(),
    });
    (
        AxumStatus::OK,
        "{\"data\":{\"guardrailsResult\":{\"Allowed\":true}}}",
    )
}

async fn serve_router(router: Router) -> (String, JoinSet<std::io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

async fn serve(captures: Captures) -> (String, JoinSet<std::io::Result<()>>) {
    let router = Router::new()
        .route("/api/http-proxy", post(capture))
        .with_state(captures);
    serve_router(router).await
}

// Hardcoded independently of MAX_RESPONSE_BYTES, so a mutation of that constant can't move this boundary too.
const SPEC_MAX_RESPONSE_BYTES: usize = 1_048_576;

fn padded_body(size: usize) -> String {
    let base = "{\"data\":{\"guardrailsResult\":{\"Allowed\":true}}}";
    let mut body = base.to_owned();
    body.push_str(&" ".repeat(size.saturating_sub(base.len())));
    body
}

async fn serve_fixed_body(body: String) -> (String, JoinSet<std::io::Result<()>>) {
    let router = Router::new().route(
        "/api/http-proxy",
        post(move || {
            let body = body.clone();
            async move { (AxumStatus::OK, body) }
        }),
    );
    serve_router(router).await
}

async fn send_to_body(body: String) -> AktoAnswer {
    let (base, _server) = serve_fixed_body(body).await;
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    guardrail
        .send_request(true, false, &payload)
        .await
        .expect("answer")
}

#[tokio::test]
async fn akto_guardrail_send_request_accepts_a_body_at_the_limit() {
    let answer = send_to_body(padded_body(SPEC_MAX_RESPONSE_BYTES)).await;
    assert_eq!(answer.body.len(), SPEC_MAX_RESPONSE_BYTES);
    let verdict = AktoMirroring::handle_guardrail_response(&answer).expect("verdict");
    assert!(verdict.allowed);
}

#[tokio::test]
async fn akto_guardrail_send_request_accepts_a_body_over_2048_bytes() {
    let answer = send_to_body(padded_body(3000)).await;
    assert_eq!(answer.body.len(), 3000);
    let verdict = AktoMirroring::handle_guardrail_response(&answer).expect("verdict");
    assert!(verdict.allowed);
}

#[tokio::test]
async fn akto_guardrail_send_request_refuses_a_body_over_the_limit() {
    let (base, _server) = serve_fixed_body(padded_body(SPEC_MAX_RESPONSE_BYTES + 1)).await;
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let error = guardrail
        .send_request(true, false, &payload)
        .await
        .expect_err("response too large");
    assert!(
        matches!(error, AktoError::ResponseTooLarge { limit } if limit == SPEC_MAX_RESPONSE_BYTES)
    );
}

#[tokio::test]
async fn akto_guardrail_send_request_posts_the_mirroring_payload() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let (base, _server) = serve(Arc::clone(&captures)).await;
    let mut guardrail = guardrail_at(fixed_clock);
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let answer = guardrail
        .send_request(true, false, &payload)
        .await
        .expect("answer");
    assert_eq!(answer.status, StatusCode::OK);
    let recorded = captures.lock().expect("captures").pop().expect("request");
    assert_eq!(recorded.path, "/api/http-proxy");
    assert_eq!(
        recorded.query.as_deref(),
        Some("akto_connector=litellm&guardrails=true")
    );
    assert_eq!(recorded.content_type.as_deref(), Some("application/json"));
    assert_eq!(recorded.authorization.as_deref(), Some("secret"));
    let sent: AktoPayload = serde_json::from_slice(&recorded.body).expect("sent payload");
    assert_eq!(sent, payload);
}

#[tokio::test]
async fn akto_guardrail_send_request_parses_the_verdict_after_sending() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let (base, _server) = serve(Arc::clone(&captures)).await;
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let answer = guardrail
        .send_request(false, true, &payload)
        .await
        .expect("answer");
    let verdict = AktoMirroring::handle_guardrail_response(&answer).expect("verdict");
    assert!(verdict.allowed);
    assert!(verdict.reason.is_empty());
    let recorded = captures.lock().expect("captures").pop().expect("request");
    assert_eq!(
        recorded.query.as_deref(),
        Some("akto_connector=litellm&ingest_data=true")
    );
}

#[tokio::test]
async fn akto_guardrail_fire_and_forget_swallows_an_error_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let router = Router::new().route(
        "/api/http-proxy",
        post(|| async { (AxumStatus::INTERNAL_SERVER_ERROR, "oops") }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let path = log_path("fire-and-forget-error");
    {
        let _guard = log_to(&path);
        guardrail
            .fire_and_forget_request(true, true, &payload)
            .await;
    }
    let written = fs::read_to_string(&path).expect("log contents");
    fs::remove_file(&path).expect("remove log");
    assert!(written.contains("non-200"));
}

#[tokio::test]
async fn akto_guardrail_fire_and_forget_sends_the_request() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let (base, _server) = serve(Arc::clone(&captures)).await;
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let path = log_path("fire-and-forget-success");
    {
        let _guard = log_to(&path);
        guardrail
            .fire_and_forget_request(true, true, &payload)
            .await;
    }
    let recorded = captures.lock().expect("captures").pop().expect("request");
    assert_eq!(
        recorded.query.as_deref(),
        Some("akto_connector=litellm&guardrails=true&ingest_data=true")
    );
    let written = fs::read_to_string(&path).expect("log contents");
    fs::remove_file(&path).expect("remove log");
    assert!(!written.contains("non-200"));
}

#[tokio::test]
async fn akto_guardrail_fire_and_forget_swallows_a_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let mut guardrail = guardrail();
    guardrail.base_url = format!("http://{address}");
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    guardrail
        .fire_and_forget_request(true, true, &payload)
        .await;
}

#[tokio::test]
async fn akto_guardrail_tracks_background_ingest_tasks_until_they_finish() {
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let (base, _server) = serve(Arc::clone(&captures)).await;
    let mut guardrail = guardrail();
    guardrail.base_url = base;
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, true)
        .expect("payload");
    let background = guardrail.clone();
    guardrail.tasks.spawn(async move {
        background
            .fire_and_forget_request(true, true, &payload)
            .await;
    });
    assert_eq!(guardrail.tasks.len(), 1);
    guardrail.tasks.close();
    guardrail.tasks.wait().await;
    assert!(guardrail.tasks.is_empty());
    assert_eq!(captures.lock().expect("captures").len(), 1);
}
