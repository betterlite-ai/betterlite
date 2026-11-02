#![cfg(test)]

use std::error::Error as _;
use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use reqwest::redirect::Policy;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::transport::{build_headers, post_inspection};

fn configured(timeout: f64) -> Configured {
    Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("secret-key".to_owned()),
        api_base: "https://cisco.test".to_owned(),
        inspection_type: InspectionType::Chat,
        inspect_path: "/api/v1/inspect/chat".to_owned(),
        enabled_rules: Vec::new(),
        on_flagged_action: FlaggedAction::Block,
        fallback_on_error: FallbackAction::Block,
        timeout,
        integration_profile_id: None,
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    }
}

fn plain_client() -> reqwest::Client {
    reqwest::Client::default()
}

fn client_without_redirects() -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .redirect(Policy::none())
        .build()
        .expect("client")
}

fn guardrail(timeout: f64, client: reqwest::Client) -> CiscoInspectionGuardrail {
    CiscoInspectionGuardrail {
        settings: configured(timeout),
        client,
    }
}

async fn serve(status: StatusCode, body: Vec<u8>) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/inspect",
        post(move || {
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

#[test]
fn build_headers_sets_the_cisco_api_key_header() {
    let guardrail = guardrail(10.0, plain_client());
    let headers = build_headers(&guardrail).expect("headers");
    assert_eq!(
        headers.get("x-cisco-ai-defense-api-key").expect("key"),
        "secret-key"
    );
    assert_eq!(
        headers.get("content-type").expect("content-type"),
        "application/json"
    );
    assert_eq!(headers.get("accept").expect("accept"), "application/json");
    assert_eq!(
        headers.get("user-agent").expect("user-agent"),
        "litellm/1.105.1"
    );
}

#[tokio::test]
async fn post_inspection_parses_a_successful_json_response() {
    let (base, mut server) = serve(StatusCode::OK, br#"{"is_safe":true}"#.to_vec()).await;
    let guardrail = guardrail(10.0, plain_client());
    let result = post_inspection(&guardrail, &format!("{base}/inspect"), &(), "chat").await;
    result.expect("successful response");
    server.shutdown().await;
}

#[tokio::test]
async fn post_inspection_wraps_a_non_success_status() {
    let (base, mut server) = serve(StatusCode::BAD_REQUEST, b"bad input".to_vec()).await;
    let guardrail = guardrail(10.0, plain_client());
    let error = post_inspection(&guardrail, &format!("{base}/inspect"), &(), "chat")
        .await
        .expect_err("non-success status");
    assert!(error.to_string().contains("HTTP 400"));
    server.shutdown().await;
}

#[tokio::test]
async fn post_inspection_wraps_a_non_json_body() {
    let (base, mut server) = serve(StatusCode::OK, b"not json".to_vec()).await;
    let guardrail = guardrail(10.0, plain_client());
    let error = post_inspection(&guardrail, &format!("{base}/inspect"), &(), "chat")
        .await
        .expect_err("non-JSON body");
    assert!(error.to_string().contains("non-JSON response"));
    server.shutdown().await;
}

#[tokio::test]
async fn post_inspection_wraps_a_connection_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}/inspect", listener.local_addr().expect("addr"));
    drop(listener);
    let guardrail = guardrail(10.0, plain_client());
    let error = post_inspection(&guardrail, &base, &(), "chat")
        .await
        .expect_err("connection refused");
    assert!(error.to_string().contains("request failed"));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn post_inspection_wraps_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}/inspect", listener.local_addr().expect("addr"));
    let guardrail = guardrail(0.2, plain_client());
    let error = post_inspection(&guardrail, &base, &(), "chat")
        .await
        .expect_err("timeout");
    assert!(error.to_string().contains("timed out after 0.2s"));
    assert!(error.source().is_some());
    drop(listener);
}

#[tokio::test]
async fn post_inspection_does_not_follow_redirects() {
    let (base, mut server) = serve(StatusCode::FOUND, Vec::new()).await;
    let guardrail = guardrail(10.0, client_without_redirects());
    let error = post_inspection(&guardrail, &format!("{base}/inspect"), &(), "chat")
        .await
        .expect_err("redirect surfaced as non-success");
    assert!(error.to_string().contains("HTTP 302"));
    server.shutdown().await;
}

#[tokio::test]
async fn post_inspection_wraps_a_response_over_the_byte_limit() {
    let oversized = vec![b'a'; (1024 * 1024) + 1];
    let (base, mut server) = serve(StatusCode::OK, oversized).await;
    let guardrail = guardrail(10.0, plain_client());
    let error = post_inspection(&guardrail, &format!("{base}/inspect"), &(), "chat")
        .await
        .expect_err("oversized response");
    assert!(error.to_string().contains("exceeds 1048576 bytes"));
    server.shutdown().await;
}

#[tokio::test]
async fn post_inspection_renders_a_whole_number_timeout_with_a_trailing_zero() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}/inspect", listener.local_addr().expect("addr"));
    let guardrail = guardrail(1.0, plain_client());
    let error = post_inspection(&guardrail, &base, &(), "chat")
        .await
        .expect_err("timeout");
    assert!(error.to_string().contains("timed out after 1.0s"));
    drop(listener);
}
