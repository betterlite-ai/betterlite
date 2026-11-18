#![cfg(test)]

use axum::http::Method;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;

use crate::roi_connections::gateway_http_client::{GATEWAY_TIMEOUT, GatewayError};
use crate::roi_connections::tests::support::{Answer, Wire, gateway, secret};

#[tokio::test]
async fn gateway_http_client_posts_the_bearer_credential() {
    let wire = Wire::new(vec![Answer::ok("{\"ok\":true}")]);
    let response = gateway(&wire)
        .request(
            Method::POST,
            "/v1/chat/completions",
            &secret("sk-estimator"),
            RequestBody::Json(b"{}".to_vec()),
        )
        .await
        .expect("gateway answer");
    assert_eq!(response.status, 200);
    let seen = wire.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "http://litellm.internal/v1/chat/completions");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer sk-estimator")
    );
    assert_eq!(seen[0].deadline, GATEWAY_TIMEOUT);
    assert_eq!(seen[0].body, "{}".as_bytes().to_vec());
}

#[tokio::test]
async fn gateway_http_client_returns_a_redirect_answer() {
    let wire = Wire::new(vec![
        Answer::status(302).with_header("location", "/elsewhere"),
    ]);
    let response = gateway(&wire)
        .request(
            Method::GET,
            "/v1/models",
            &secret("sk-estimator"),
            RequestBody::Empty,
        )
        .await
        .expect("redirect answer");
    assert_eq!(response.status, 302);
    assert_eq!(wire.seen().len(), 1);
}

#[tokio::test]
async fn gateway_http_client_reports_an_error_status() {
    let wire = Wire::new(vec![Answer::status(500)]);
    let error = gateway(&wire)
        .request(
            Method::GET,
            "/v1/models",
            &secret("sk-estimator"),
            RequestBody::Empty,
        )
        .await
        .err()
        .expect("gateway failure");
    assert!(matches!(error, GatewayError::Status { status: 500 }));
}

#[tokio::test]
async fn gateway_http_client_reports_a_transport_failure() {
    let wire = Wire::new(Vec::new());
    let error = gateway(&wire)
        .request(Method::GET, "/v1/models", &secret("sk"), RequestBody::Empty)
        .await
        .err()
        .expect("gateway failure");
    assert!(matches!(
        error,
        GatewayError::Transport(TransportError::Closed)
    ));
}

#[tokio::test]
async fn gateway_http_client_reports_an_unreachable_path() {
    let wire = Wire::new(Vec::new());
    let error = gateway(&wire)
        .request(Method::GET, "http://[", &secret("sk"), RequestBody::Empty)
        .await
        .err()
        .expect("gateway failure");
    assert!(matches!(error, GatewayError::Url(_)));
}

#[tokio::test]
async fn gateway_http_client_reports_an_invalid_credential() {
    let wire = Wire::new(Vec::new());
    let error = gateway(&wire)
        .request(
            Method::GET,
            "/v1/models",
            &secret("bad\nkey"),
            RequestBody::Empty,
        )
        .await
        .err()
        .expect("gateway failure");
    assert!(matches!(error, GatewayError::Header(_)));
}
