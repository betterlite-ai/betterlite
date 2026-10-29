#![cfg(test)]
use crate::error_body::{AuthenticationFailure, authentication_error, json_error};
use axum::body::to_bytes;
use axum::http::StatusCode;

#[tokio::test]
async fn json_error_carries_status_and_fields() {
    let response = json_error(
        StatusCode::BAD_REQUEST,
        "bad input",
        "invalid_request_error",
        "400",
    );
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8(body.to_vec()).unwrap(),
        r#"{"error":{"message":"bad input","type":"invalid_request_error","param":null,"code":"400"}}"#
    );
}

#[tokio::test]
async fn authentication_error_missing_key_is_internal_server_error() {
    let response = authentication_error(&AuthenticationFailure::Missing);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8(body.to_vec()).unwrap(),
        "Internal Server Error"
    );
}

#[tokio::test]
async fn authentication_error_invalid_key_is_bad_request() {
    let response = authentication_error(&AuthenticationFailure::Invalid);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8(body.to_vec()).unwrap(),
        r#"{"error":{"message":"the API key is not valid","type":"no_db_connection","param":null,"code":"400"}}"#
    );
}
