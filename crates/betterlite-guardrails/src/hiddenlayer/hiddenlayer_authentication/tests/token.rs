use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::limits::TransportLimits;
use secrecy::SecretString;

use super::super::token::{JwtError, JwtTransport as _, ReqwestJwtTransport, TokenRequest};
use super::support;

const TOKEN_PATH: &str = "/oauth2/token?grant_type=client_credentials";
const UNREACHABLE: &str = "http://127.0.0.1:1";

fn request(url: &str, timeout: Duration) -> TokenRequest {
    TokenRequest {
        url: url.to_owned(),
        api_id: "client".to_owned(),
        api_key: SecretString::from("secret"),
        timeout,
    }
}

fn ask(server: &support::Server) -> Result<String, JwtError> {
    let url = format!("{}{TOKEN_PATH}", server.base);
    ReqwestJwtTransport.fetch_token(&request(&url, Duration::from_secs(5)))
}

#[test]
fn the_client_credentials_are_sent_as_basic_auth() {
    let server = support::start(vec![support::reply(200, r#"{"access_token":"token-1"}"#)]);
    let token = ask(&server).unwrap();
    assert_eq!(token, "token-1");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, TOKEN_PATH);
    let expected = format!("Basic {}", STANDARD.encode("client:secret"));
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn a_non_ok_status_is_an_error() {
    let server = support::start(vec![support::reply(401, r#"{"error":"denied"}"#)]);
    let error = ask(&server).unwrap_err();
    let JwtError::Status { status, body } = error else {
        panic!("a status error was expected, got {error}");
    };
    assert_eq!(status, 401);
    assert_eq!(body, r#"{"error":"denied"}"#);
}

#[test]
fn a_bad_request_status_is_an_error() {
    let server = support::start(vec![support::reply(400, "bad request")]);
    let error = ask(&server).unwrap_err();
    assert!(matches!(error, JwtError::Status { status: 400, .. }));
}

#[test]
fn a_redirect_class_status_is_answered() {
    let server = support::start(vec![support::reply(399, r#"{"access_token":"token-1"}"#)]);
    let token = ask(&server).unwrap();
    assert_eq!(token, "token-1");
}

#[test]
fn a_missing_access_token_is_an_invalid_response() {
    let server = support::start(vec![support::reply(200, r#"{"user":"client"}"#)]);
    let error = ask(&server).unwrap_err();
    assert!(matches!(error, JwtError::InvalidResponse { .. }));
    assert_eq!(
        error.to_string(),
        r#"Unable to get authentication credentials for the HiddenLayer API - invalid response: {"user":"client"}"#
    );
}

#[test]
fn a_non_text_access_token_is_an_invalid_response() {
    let server = support::start(vec![support::reply(200, r#"{"access_token":5}"#)]);
    let error = ask(&server).unwrap_err();
    assert!(matches!(error, JwtError::InvalidResponse { .. }));
}

#[test]
fn an_empty_access_token_is_kept() {
    let server = support::start(vec![support::reply(200, r#"{"access_token":""}"#)]);
    let token = ask(&server).unwrap();
    assert_eq!(token, "");
}

#[test]
fn a_non_json_token_body_is_a_decode_error() {
    let server = support::start(vec![support::reply(200, "not json")]);
    let error = ask(&server).unwrap_err();
    assert!(matches!(error, JwtError::Decode { .. }));
}

#[test]
fn a_token_body_at_the_transport_limit_is_read() {
    let limit = TransportLimits::DEFAULT.body_bytes;
    let prefix = r#"{"access_token":""#;
    let suffix = "\"}";
    let padding = "a".repeat(limit - prefix.len() - suffix.len());
    let body = format!("{prefix}{padding}{suffix}");
    assert_eq!(body.len(), limit);
    let server = support::start(vec![support::reply(200, &body)]);
    let token = ask(&server).unwrap();
    assert_eq!(token, padding);
}

#[test]
fn a_token_body_past_the_transport_limit_is_an_error() {
    let limit = TransportLimits::DEFAULT.body_bytes;
    let oversized = "a".repeat(limit + 1);
    let server = support::start(vec![support::reply(200, &oversized)]);
    let error = ask(&server).unwrap_err();
    assert!(matches!(error, JwtError::TooLarge { limit: reported } if reported == limit));
}

#[test]
fn a_transport_failure_is_a_transport_error() {
    let url = format!("{UNREACHABLE}{TOKEN_PATH}");
    let error = ReqwestJwtTransport
        .fetch_token(&request(&url, Duration::from_secs(5)))
        .unwrap_err();
    assert!(matches!(error, JwtError::Transport { .. }));
}

#[test]
fn the_request_timeout_is_applied() {
    let server = support::start(vec![support::delayed(
        200,
        r#"{"access_token":"late"}"#,
        Duration::from_millis(300),
    )]);
    let url = format!("{}{TOKEN_PATH}", server.base);
    let error = ReqwestJwtTransport
        .fetch_token(&request(&url, Duration::from_millis(20)))
        .unwrap_err();
    assert!(matches!(error, JwtError::Transport { .. }));
}
