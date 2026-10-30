#![cfg(test)]

use axum::body::to_bytes;
use axum::http::{HeaderMap, StatusCode};
use betterlite_http::proxy::UpstreamResponse;

use super::{EmbeddingError, failure};

#[test]
fn embedding_failure_returns_bad_request() {
    let error = EmbeddingError::Body {
        source: serde_json::from_str::<String>("not-json").expect_err("invalid JSON"),
    };
    let response = failure(&error, "embedding-test", None);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get("x-litellm-call-id").unwrap(),
        "embedding-test"
    );
}

#[tokio::test]
async fn embedding_provider_failure_preserves_message() {
    let error = EmbeddingError::Provider {
        response: Box::new(UpstreamResponse {
            status: 400,
            headers: HeaderMap::new(),
            body: br#"{"error":{"message":"upstream bad","type":"invalid_request_error"}}"#
                .to_vec(),
        }),
    };
    let response = failure(&error, "embedding-provider", None);
    let body = to_bytes(response.into_body(), 4096)
        .await
        .expect("error body");
    assert!(body.windows(12).any(|window| window == b"upstream bad"));
}
