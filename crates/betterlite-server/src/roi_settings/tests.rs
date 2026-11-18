#![cfg(test)]

use axum::http::StatusCode;

use super::support::call;
use super::support::get_request;
use super::support::put_request;
use super::support::settings_app;

#[tokio::test]
async fn oneshot_get_without_a_caller_is_unauthorized() {
    let reply = call(settings_app(None, None), get_request(None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oneshot_put_without_a_caller_is_unauthorized() {
    let reply = call(
        settings_app(None, None),
        put_request(None, r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}
