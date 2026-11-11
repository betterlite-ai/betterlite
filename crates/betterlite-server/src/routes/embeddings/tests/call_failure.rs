use axum::body::to_bytes;
use axum::http::StatusCode;
use betterlite_provider::operation::OperationError;
use betterlite_provider::transform_retry::TransformRetryError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde::Deserialize;

use super::super::{CallError, EmbeddingError, call_failure};

#[derive(Deserialize)]
struct Reply {
    error: Detail,
}

#[derive(Deserialize)]
struct Detail {
    #[serde(rename = "type")]
    error_type: String,
    code: String,
}

#[rstest]
#[case::mismatched_operation(CallError::Mismatched)]
#[case::transform_retry(CallError::Operation {
    source: OperationError::TransformRetry(TransformRetryError::Attempts { max_attempts: 0 }),
})]
#[tokio::test]
async fn provider_framework_tests_behavior(#[case] error: CallError<EmbeddingError>) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let response = call_failure(&error, "call-7", None, &redactor);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let reply: Reply = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply.error.error_type, "internal_server_error");
    assert_eq!(reply.error.code, "500");
}
