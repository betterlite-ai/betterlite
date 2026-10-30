#![cfg(test)]

use axum::http::StatusCode;

use super::{EmbeddingError, failure};

#[test]
fn embedding_failure_returns_bad_request() {
    let error = EmbeddingError::Body {
        source: serde_json::from_str::<String>("not-json").expect_err("invalid JSON"),
    };
    let response = failure(&error, "embedding-test", None);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
