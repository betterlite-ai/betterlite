#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

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

async fn serve(
    status: StatusCode,
    body: String,
) -> (String, mpsc::Receiver<Captured>, JoinSet<io::Result<()>>) {
    let (sender, receiver) = mpsc::channel(1);
    let router = Router::new().route(
        "/api/v1/protect",
        post(move |uri: Uri, headers: HeaderMap, request_body: String| {
            let request_sender = sender.clone();
            let reply = body.clone();
            async move {
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
        messages: GuardrailValue::Array(vec![GuardrailValue::Object(
            [("role".to_owned(), GuardrailValue::Text("user".to_owned()))]
                .into_iter()
                .collect(),
        )]),
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

#[tokio::test]
async fn the_call_posts_the_payload_to_the_protect_route() {
    let (base, receiver, mut server) = serve(
        StatusCode::OK,
        r#"{"flagged": false, "session_id": "s1"}"#.to_owned(),
    )
    .await;
    let client = reqwest::Client::new();
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
            .and_then(|v| v.to_str().ok()),
        Some("Bearer sk-test")
    );
    assert_eq!(
        captured
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let body: serde_json::Value = serde_json::from_str(&captured.body).expect("valid JSON body");
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["provider"], "openai");
    assert_eq!(body["metadata"]["source"], "litellm");
    assert_eq!(body["messages"][0]["role"], "user");
    server.shutdown().await;
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::not_found(StatusCode::NOT_FOUND)]
#[case::bad_request(StatusCode::BAD_REQUEST)]
#[tokio::test]
async fn a_non_success_status_is_an_error(#[case] status: StatusCode) {
    let (base, _receiver, mut server) = serve(status, String::from("nope")).await;
    let client = reqwest::Client::new();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("a non-success status is an error");
    assert!(matches!(error, PillarError::Status { status: named } if named == status.as_u16()));
    server.shutdown().await;
}

#[test]
fn a_body_that_is_not_json_is_an_error() {
    let body = super::api_call::parse_protect_response(b"not json")
        .expect_err("an unparsable body is an error");
    assert!(matches!(body, PillarError::Response { .. }));
}

#[test]
fn a_body_of_the_wrong_type_is_an_error() {
    let body = super::api_call::parse_protect_response(br#"["flagged"]"#)
        .expect_err("a non-object body is an error");
    assert!(matches!(body, PillarError::Response { .. }));
}

#[test]
fn unknown_response_keys_are_kept_so_the_body_is_not_treated_as_empty() {
    let parsed =
        super::api_call::parse_protect_response(br#"{"unknown": 1}"#).expect("the body parses");
    assert_ne!(parsed, ProtectResponse::default());
    assert_eq!(parsed.other.len(), 1);
}

#[test]
fn an_empty_object_is_empty() {
    let parsed = super::api_call::parse_protect_response(b"{}").expect("the body parses");
    assert_eq!(parsed, ProtectResponse::default());
}

#[tokio::test]
async fn a_body_at_the_cap_is_accepted() {
    let prefix = "{\"pad\":\"";
    let suffix = "\"}";
    let pad = 1024 * 1024 - prefix.len() - suffix.len();
    let body = format!("{prefix}{}{suffix}", "x".repeat(pad));
    assert_eq!(body.len(), 1024 * 1024);
    let (base, _receiver, mut server) = serve(StatusCode::OK, body).await;
    let client = reqwest::Client::new();
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
    let client = reqwest::Client::new();
    let error = call_pillar_api(&client, &pillar_settings(&base), &headers(), &payload())
        .await
        .expect_err("a body over the cap is an error");
    assert!(matches!(error, PillarError::ResponseTooLarge { .. }));
    server.shutdown().await;
}

#[tokio::test]
async fn a_refused_connection_is_a_request_error() {
    let settings = pillar_settings("http://127.0.0.1:1");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("client");
    let error = call_pillar_api(&client, &settings, &headers(), &payload())
        .await
        .expect_err("a refused connection is an error");
    assert!(matches!(error, PillarError::Request { .. }));
}

#[tokio::test]
async fn an_invalid_header_value_is_rejected_before_the_call() {
    let (base, _receiver, mut server) = serve(StatusCode::OK, String::from("{}")).await;
    let bad = BTreeMap::from([("Authorization".to_owned(), "Bearer a\nb".to_owned())]);
    let client = reqwest::Client::new();
    let error = call_pillar_api(&client, &pillar_settings(&base), &bad, &payload())
        .await
        .expect_err("a header with a newline is invalid");
    assert!(matches!(error, PillarError::InvalidHeaderValue { .. }));
    server.shutdown().await;
}
