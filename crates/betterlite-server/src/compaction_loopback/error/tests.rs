#![cfg(test)]

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde::Deserialize;

use super::LoopbackError;

#[derive(Deserialize)]
struct Envelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Json,
    code: String,
}

#[rstest]
#[case(LoopbackError::ChildFailed { status: 502 })]
#[case(LoopbackError::InvalidChildBody)]
#[case(LoopbackError::MissingModel)]
#[tokio::test]
async fn every_loopback_error_answers_400_invalid_request_error(#[case] error: LoopbackError) {
    let expected = error.to_string();
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let envelope: Envelope = serde_json::from_slice(&body).unwrap();
    assert_eq!(envelope.error.message, expected);
    assert_eq!(envelope.error.error_type, "invalid_request_error");
    assert_eq!(envelope.error.code, "400");
    assert_eq!(envelope.error.param, Json::Null);
}
