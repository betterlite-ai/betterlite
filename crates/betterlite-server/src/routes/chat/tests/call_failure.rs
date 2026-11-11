#![cfg(test)]

use std::time::Instant;

use axum::body::to_bytes;
use axum::http::StatusCode;
use betterlite_error::normalize::Normalizer;
use betterlite_openai::chat::execution::ChatError;
use betterlite_provider::operation::OperationError;
use betterlite_provider::transform_retry::TransformRetryError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde::Deserialize;

use crate::routes::chat::{CallContext, ErrorLayer, call_failure};
use crate::server::chat_setup::provider_call::CallError;

#[derive(Deserialize)]
struct Reply {
    error: Detail,
}

#[derive(Deserialize)]
struct Detail {
    #[serde(rename = "type")]
    error_type: String,
    code: String,
    litellm_call_id: Option<String>,
}

#[rstest]
#[case::mismatched_operation(CallError::Mismatched)]
#[case::transform_retry(CallError::Operation {
    source: OperationError::TransformRetry(TransformRetryError::Attempts { max_attempts: 0 }),
})]
#[tokio::test]
async fn provider_framework_tests_behavior(#[case] error: CallError<ChatError>) {
    let layer = ErrorLayer {
        normalizer: Normalizer::new().unwrap(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let call = CallContext {
        id: "call-7",
        include_in_body: true,
        started: Instant::now(),
        errors: &layer,
    };
    let response = call_failure(&error, &call, None);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let reply: Reply = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply.error.error_type, "internal_server_error");
    assert_eq!(reply.error.code, "500");
    assert_eq!(reply.error.litellm_call_id.as_deref(), Some("call-7"));
}
