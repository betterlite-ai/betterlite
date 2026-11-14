use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_http::proxy::base_builder;
use indexmap::IndexMap;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::primitives::json_value::JsonValue;

use super::super::HiddenlayerAuth;
use super::super::detection::{DetectionRequest, DetectionResponse};
use super::super::error::HiddenlayerError;
use super::super::settings::HiddenlayerCredentials;
use super::super::token::{JwtError, JwtTransport};
use super::support::{self, TokenScript};

const SAAS_HOST: &str = "api.hiddenlayer.ai";
const PROJECT_HEADER: &str = "hl-project-id";

fn on_prem(base: &str, script: &Arc<TokenScript>) -> HiddenlayerAuth {
    let input = HiddenlayerCredentials {
        api_id: None,
        api_key: None,
        api_base: Some(base.to_owned()),
        auth_url: None,
        timeout: None,
    };
    let client = base_builder().build().unwrap();
    let transport: Arc<dyn JwtTransport> = Arc::<TokenScript>::clone(script);
    HiddenlayerAuth::new(&input, &BTreeMap::new(), client, transport).unwrap()
}

fn saas(port: u16, auth_url: &str, script: &Arc<TokenScript>) -> HiddenlayerAuth {
    let input = HiddenlayerCredentials {
        api_id: Some("client".to_owned()),
        api_key: Some(SecretString::from("secret")),
        api_base: Some(format!("http://{SAAS_HOST}:{port}")),
        auth_url: Some(auth_url.to_owned()),
        timeout: None,
    };
    let client = base_builder()
        .resolve(SAAS_HOST, SocketAddr::from(([127, 0, 0, 1], port)))
        .build()
        .unwrap();
    let transport: Arc<dyn JwtTransport> = Arc::<TokenScript>::clone(script);
    HiddenlayerAuth::new(&input, &BTreeMap::new(), client, transport).unwrap()
}

fn payload() -> JsonValue {
    JsonValue::Object(IndexMap::from([(
        "input".to_owned(),
        JsonValue::String("hello".to_owned()),
    )]))
}

fn caller_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(PROJECT_HEADER, HeaderValue::from_static("project-1"));
    headers
}

fn request<'request>(path: &'request str, body: &'request JsonValue) -> DetectionRequest<'request> {
    DetectionRequest {
        path,
        headers: caller_headers(),
        body,
    }
}

fn response(result: Result<DetectionResponse, HiddenlayerError>) -> DetectionResponse {
    match result {
        Ok(response) => response,
        Err(error) => panic!("the detection request must succeed: {error}"),
    }
}

fn failure(result: Result<DetectionResponse, HiddenlayerError>) -> HiddenlayerError {
    match result {
        Ok(response) => panic!("the detection request must fail: {response:?}"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn detection_sends_the_payload_with_the_caller_headers() {
    let server = support::start(vec![support::reply(200, r#"{"ok":true}"#)]);
    let script = support::tokens(&[]);
    let auth = on_prem(&server.base, &script);
    let body = payload();
    let answered = response(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert_eq!(answered.status, 200);
    assert_eq!(answered.body.as_slice(), br#"{"ok":true}"#.as_slice());
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/detection/v1/interactions");
    assert_eq!(requests[0].authorization, None);
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
    let sent: JsonValue = serde_json::from_str(&requests[0].body).unwrap();
    assert_eq!(sent, body);
}

#[tokio::test]
async fn detection_sends_the_bearer_token() {
    let server = support::start(vec![support::reply(200, "{}")]);
    let script = support::tokens(&["token-1"]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    response(
        auth.post_detection(&request("detection/v2/request-evaluations", &body))
            .await,
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].path, "/detection/v2/request-evaluations",
        "the versioned path the caller chose is used as is"
    );
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer token-1"));
}

#[tokio::test]
async fn detection_refreshes_the_token_on_401_and_retries() {
    let server = support::start(vec![
        support::reply(401, "expired"),
        support::reply(200, r#"{"evaluation":{}}"#),
    ]);
    let script = support::tokens(&["token-1", "token-2"]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    let answered = response(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert_eq!(answered.status, 200);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer token-1"));
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer token-2"));
    let calls = script.requests();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].url,
        format!("{}/oauth2/token?grant_type=client_credentials", server.base)
    );
    assert_eq!(calls[1].api_id, "client");
    assert_eq!(calls[1].api_key, "secret");
}

#[tokio::test]
async fn a_refreshed_token_is_reused_by_later_requests() {
    let server = support::start(vec![
        support::reply(401, "expired"),
        support::reply(200, "{}"),
        support::reply(200, "{}"),
    ]);
    let script = support::tokens(&["token-1", "token-2"]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    response(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    response(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].authorization.as_deref(), Some("Bearer token-2"));
    assert_eq!(script.requests().len(), 2);
}

#[tokio::test]
async fn detection_refreshes_only_once() {
    let server = support::start(vec![
        support::reply(401, "expired"),
        support::reply(401, "still expired"),
    ]);
    let script = support::tokens(&["token-1", "token-2"]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        error,
        HiddenlayerError::Status { status: 401, .. }
    ));
    assert_eq!(server.requests().len(), 2);
    assert_eq!(script.requests().len(), 2);
}

#[tokio::test]
async fn detection_does_not_refresh_without_a_token() {
    let server = support::start(vec![support::reply(401, "expired")]);
    let script = support::tokens(&[]);
    let auth = on_prem(&server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        error,
        HiddenlayerError::Status { status: 401, .. }
    ));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].authorization, None);
    assert!(script.requests().is_empty());
}

#[tokio::test]
async fn detection_does_not_retry_other_statuses() {
    let server = support::start(vec![support::reply(403, "forbidden")]);
    let script = support::tokens(&["token-1"]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        error,
        HiddenlayerError::Status { status: 403, .. }
    ));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        script.requests().len(),
        1,
        "only the construction-time fetch happened"
    );
}

#[tokio::test]
async fn detection_returns_the_error_body() {
    let server = support::start(vec![support::reply(500, "boom")]);
    let script = support::tokens(&[]);
    let auth = on_prem(&server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        &error,
        HiddenlayerError::Status { status: 500, body: reported } if reported.as_str() == "boom"
    ));
    assert_eq!(
        error.to_string(),
        "the HiddenLayer detection endpoint returned 500: boom"
    );
}

#[tokio::test]
async fn detection_returns_the_response_headers() {
    let server = support::start(vec![support::reply_with(
        200,
        &[("hl-runtime-action", "block")],
        "{}",
    )]);
    let script = support::tokens(&[]);
    let auth = on_prem(&server.base, &script);
    let body = payload();
    let answered = response(
        auth.post_detection(&request("detection/v2/request-evaluations", &body))
            .await,
    );
    assert_eq!(
        answered
            .headers
            .get("hl-runtime-action")
            .and_then(|value| value.to_str().ok()),
        Some("block")
    );
}

#[tokio::test]
async fn detection_reports_a_transport_failure() {
    let script = support::tokens(&[]);
    let auth = on_prem("http://127.0.0.1:1", &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(error, HiddenlayerError::Transport { .. }));
}

#[tokio::test]
async fn an_empty_token_is_not_sent_but_still_refreshes() {
    let server = support::start(vec![
        support::reply(401, "expired"),
        support::reply(200, "{}"),
    ]);
    let script = support::tokens(&["", "token-2"]);
    let auth = saas(server.port, &server.base, &script);
    assert!(support::has_token(&auth));
    let body = payload();
    let answered = response(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert_eq!(answered.status, 200);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].authorization, None);
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer token-2"));
    assert_eq!(script.requests().len(), 2);
}

#[tokio::test]
async fn an_empty_refreshed_token_is_not_sent_on_the_retry() {
    let server = support::start(vec![
        support::reply(401, "expired"),
        support::reply(401, "again"),
    ]);
    let script = support::tokens(&["token-1", ""]);
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        error,
        HiddenlayerError::Status { status: 401, .. }
    ));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer token-1"));
    assert_eq!(requests[1].authorization, None);
}

#[tokio::test]
async fn a_failed_refresh_fails_the_detection_request() {
    let server = support::start(vec![support::reply(401, "expired")]);
    let script = support::tokens(&["token-1"]);
    script
        .answers
        .lock()
        .unwrap()
        .push_back(Err(JwtError::InvalidResponse {
            body: "{}".to_owned(),
        }));
    let auth = saas(server.port, &server.base, &script);
    let body = payload();
    let error = failure(
        auth.post_detection(&request("detection/v1/interactions", &body))
            .await,
    );
    assert!(matches!(
        error,
        HiddenlayerError::Jwt(JwtError::InvalidResponse { .. })
    ));
    assert_eq!(server.requests().len(), 1);
}
