#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;

use super::shared::spawn_mock;
use crate::presidio_analysis::MAX_RESPONSE_BYTES;
use crate::presidio_analysis::body::read_capped_body;
use crate::presidio_analysis::error::PresidioAnalysisError;

async fn send(base: &str) -> reqwest::Response {
    reqwest::Client::default()
        .post(format!("{base}/analyze"))
        .body(r#"{"text":"x"}"#)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn response_cap_rejects_a_body_past_the_limit() {
    let mock = spawn_mock(
        |_| (StatusCode::OK, "application/json", "0123456789".to_owned()),
        Duration::ZERO,
    )
    .await;
    let error = read_capped_body(send(&mock.base).await, 4)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PresidioAnalysisError::ResponseTooLarge { limit: 4 }
    ));
}

#[tokio::test]
async fn response_cap_keeps_a_body_at_the_limit() {
    let mock = spawn_mock(
        |_| (StatusCode::OK, "application/json", "0123456789".to_owned()),
        Duration::ZERO,
    )
    .await;
    let body = read_capped_body(send(&mock.base).await, 10).await.unwrap();
    assert_eq!(body, b"0123456789");
}

#[test]
fn response_cap_is_64_mib() {
    assert_eq!(MAX_RESPONSE_BYTES, 64 * 1024 * 1024);
}
