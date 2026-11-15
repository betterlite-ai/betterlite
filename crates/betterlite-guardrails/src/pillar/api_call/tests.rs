#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::io::{self, Read as _, Write as _};
use std::net::TcpListener as StdTcpListener;
use std::path::Path;
use std::thread;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::base_builder;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::api_call::call_pillar_api;
use super::error::PillarError;
use super::payload::{ProtectMetadata, ProtectPayload};
use super::response::ProtectResponse;
use super::settings::{FallbackAction, OnFlaggedAction, PillarSettings};

struct Captured {
    method: String,
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(serde::Deserialize)]
struct CapturedPayload {
    messages: Vec<CapturedMessage>,
    metadata: CapturedMetadata,
    model: String,
    provider: String,
}

#[derive(serde::Deserialize)]
struct CapturedMessage {
    role: String,
}

#[derive(serde::Deserialize)]
struct CapturedMetadata {
    source: String,
}

async fn serve(
    status: StatusCode,
    body: String,
) -> (String, mpsc::Receiver<Captured>, JoinSet<io::Result<()>>) {
    serve_after(status, body, Duration::ZERO).await
}

async fn serve_after(
    status: StatusCode,
    body: String,
    delay: Duration,
) -> (String, mpsc::Receiver<Captured>, JoinSet<io::Result<()>>) {
    let (sender, receiver) = mpsc::channel(1);
    let router = Router::new().route(
        "/api/v1/protect",
        post(move |uri: Uri, headers: HeaderMap, request_body: String| {
            let request_sender = sender.clone();
            let reply = body.clone();
            async move {
                sleep(delay).await;
                let captured = Captured {
                    method: String::from("POST"),
                    path: uri.path().to_owned(),
                    headers,
                    body: request_body,
                };
                let _sent = request_sender.send(captured).await;
                (status, reply)
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, receiver, server)
}

fn pillar_settings(api_base: &str) -> PillarSettings {
    PillarSettings {
        api_key: None,
        api_base: api_base.to_owned(),
        on_flagged_action: OnFlaggedAction::Monitor,
        async_mode: None,
        persist_session: None,
        include_scanners: true,
        include_evidence: true,
        fallback_on_error: FallbackAction::Allow,
        timeout: Duration::from_secs(5),
    }
}

fn payload() -> ProtectPayload {
    ProtectPayload {
        messages: GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([(
            "role".to_owned(),
            GuardrailValue::Text("user".to_owned()),
        )]))]),
        tools: GuardrailValue::Array(Vec::new()),
        metadata: ProtectMetadata {
            source: "litellm",
            version: "1.105.1",
        },
        user_id: None,
        session_id: None,
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
    }
}

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Authorization".to_owned(), "Bearer sk-test".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ])
}

fn http_client() -> reqwest::Client {
    base_builder().build().expect("an outbound client")
}

#[tokio::test]
async fn the_call_posts_the_payload_to_the_protect_route() {
    let (base, mut receiver, mut server) = serve(
        StatusCode::OK,
        r#"{"flagged": false, "session_id": "s1"}"#.to_owned(),
    )
    .await;
    let client = http_client();
    let response = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect("the call succeeds");
    assert_eq!(response.flagged, Some(false));
    assert_eq!(response.session_id.as_deref(), Some("s1"));
    let captured = receiver.recv().await.expect("the mock saw the request");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/api/v1/protect");
    assert_eq!(
        captured
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-test")
    );
    assert_eq!(
        captured
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body: CapturedPayload = serde_json::from_str(&captured.body).expect("valid JSON body");
    assert_eq!(body.model, "gpt-4o");
    assert_eq!(body.provider, "openai");
    assert_eq!(body.metadata.source, "litellm");
    assert_eq!(
        body.messages.first().map(|message| message.role.as_str()),
        Some("user")
    );
    server.shutdown().await;
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::not_found(StatusCode::NOT_FOUND)]
#[case::bad_request(StatusCode::BAD_REQUEST)]
#[tokio::test]
async fn a_non_success_status_is_an_error(#[case] status: StatusCode) {
    let (base, _receiver, mut server) = serve(status, String::from("nope")).await;
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("a non-success status is an error");
    assert!(matches!(error, PillarError::Status { status: named } if named == status.as_u16()));
    server.shutdown().await;
}

#[tokio::test]
async fn a_body_that_is_not_json_is_an_error() {
    let (base, _receiver, mut server) = serve(StatusCode::OK, String::from("not json")).await;
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("an unparsable body is an error");
    assert!(matches!(error, PillarError::Response { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn a_body_of_the_wrong_type_is_an_error() {
    let (base, _receiver, mut server) = serve(StatusCode::OK, String::from(r#"["flagged"]"#)).await;
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("a non-object body is an error");
    assert!(matches!(error, PillarError::Response { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn unknown_response_keys_are_kept_so_the_body_is_not_treated_as_empty() {
    let (base, _receiver, mut server) =
        serve(StatusCode::OK, String::from(r#"{"unknown": 1}"#)).await;
    let client = http_client();
    let parsed = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect("the body parses");
    assert_ne!(parsed, ProtectResponse::default());
    assert_eq!(parsed.other.len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn an_empty_object_is_empty() {
    let (base, _receiver, mut server) = serve(StatusCode::OK, String::from("{}")).await;
    let client = http_client();
    let parsed = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect("the body parses");
    assert_eq!(parsed, ProtectResponse::default());
    server.shutdown().await;
}

#[tokio::test]
async fn a_body_at_the_cap_is_accepted() {
    let prefix = "{\"pad\":\"";
    let suffix = "\"}";
    let pad = 1024 * 1024 - prefix.len() - suffix.len();
    let body = format!("{prefix}{}{suffix}", "x".repeat(pad));
    assert_eq!(body.len(), 1024 * 1024);
    let (base, _receiver, mut server) = serve(StatusCode::OK, body).await;
    let client = http_client();
    let response = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect("a body at the cap is read");
    assert!(response.flagged.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn a_body_over_one_mib_is_rejected_before_it_is_parsed() {
    let padded = "x".repeat(1024 * 1024 + 512);
    let body = format!("{{\"pad\":\"{padded}\"}}");
    let (base, _receiver, mut server) = serve(StatusCode::OK, body).await;
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("a body over the cap is an error");
    assert!(matches!(error, PillarError::ResponseTooLarge { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn an_announced_length_over_the_cap_is_rejected_before_the_body_is_read() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let server = thread::Builder::new()
        .spawn(move || -> io::Result<()> {
            let (mut stream, _peer) = listener.accept()?;
            let mut request = [0_u8; 4096];
            stream.set_read_timeout(Some(Duration::from_millis(300)))?;
            let _read = stream.read(&mut request)?;
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2000000\r\n\r\n{}")?;
            let _drained = stream.read(&mut request);
            Ok(())
        })
        .expect("server thread");
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("an announced length over the cap is an error");
    assert!(matches!(error, PillarError::ResponseTooLarge { .. }));
    server.join().expect("server thread").expect("server io");
}

#[tokio::test]
async fn a_refused_connection_is_a_request_error() {
    let settings = pillar_settings("http://127.0.0.1:1");
    let client = base_builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("client");
    let error = call_pillar_api(&client, &settings, &headers(), &payload())
        .await
        .expect_err("a refused connection is an error");
    assert!(matches!(error, PillarError::Request { .. }));
}

#[tokio::test]
async fn a_slow_answer_past_the_timeout_is_a_request_error() {
    let (base, _receiver, mut server) = serve_after(
        StatusCode::OK,
        String::from("{}"),
        Duration::from_millis(200),
    )
    .await;
    let mut settings = pillar_settings(&base);
    settings.timeout = Duration::from_millis(50);
    let client = http_client();
    let error = call_pillar_api(&client, &settings, &headers(), &payload())
        .await
        .expect_err("an answer past the per-request timeout is an error");
    assert!(matches!(error, PillarError::Request { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn an_invalid_header_value_is_rejected_before_the_call() {
    let (base, _receiver, mut server) = serve(StatusCode::OK, String::from("{}")).await;
    let bad = BTreeMap::from([("Authorization".to_owned(), "Bearer a\nb".to_owned())]);
    let client = http_client();
    let error = call_pillar_api(&client, &pillar_settings(&base), &bad, &payload())
        .await
        .expect_err("a header with a newline is invalid");
    assert!(matches!(error, PillarError::InvalidHeaderValue { .. }));
    server.shutdown().await;
}

#[test]
fn replays_the_pillar_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/pillar_response");
    let paths: Vec<_> = fs::read_dir(corpus).expect("corpus").collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.expect("entry").path()).expect("seed file");
        serde_json::from_slice::<ProtectResponse>(&bytes).is_ok()
    }));
}
