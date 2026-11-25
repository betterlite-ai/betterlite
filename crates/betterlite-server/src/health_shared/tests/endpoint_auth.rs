#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;

use super::route::{
    SHARED_STATUS_PATH, SharedHealthSwitch, SharedStatusState, shared_status_router,
};
use super::status_view::SharedHealthStatusSource;
use super::support::{FakeStatus, caller, route_get, sample_status};

const UNAUTHORIZED_BODY: &str = r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#;

#[tokio::test]
async fn an_unauthenticated_call_is_rejected() {
    let source = FakeStatus::answering(sample_status());
    let status_source: Arc<dyn SharedHealthStatusSource> = Arc::<FakeStatus>::clone(&source);
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Enabled,
        status: Some(status_source),
    };
    let (status, text) = route_get(shared_status_router(state), None, SHARED_STATUS_PATH).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(text, UNAUTHORIZED_BODY);
    assert_eq!(source.calls(), 0);
}

#[tokio::test]
async fn the_authenticated_call_reaches_the_status_source() {
    let source = FakeStatus::answering(sample_status());
    let status_source: Arc<dyn SharedHealthStatusSource> = Arc::<FakeStatus>::clone(&source);
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Enabled,
        status: Some(status_source),
    };
    let context = caller();
    let (status, text) = route_get(
        shared_status_router(state),
        Some(&context),
        SHARED_STATUS_PATH,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        r#"{"shared_health_check_enabled":true,"status":{"pod_id":"pod_1704067200000","redis_available":true,"lock_ttl":60,"cache_ttl":300,"lock_owner":null,"lock_in_progress":false,"cache_available":false}}"#
    );
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn the_disabled_proxy_says_so_and_never_reads_redis() {
    let source = FakeStatus::answering(sample_status());
    let status_source: Arc<dyn SharedHealthStatusSource> = Arc::<FakeStatus>::clone(&source);
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Disabled,
        status: Some(status_source),
    };
    let context = caller();
    let (status, text) = route_get(
        shared_status_router(state),
        Some(&context),
        SHARED_STATUS_PATH,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        r#"{"shared_health_check_enabled":false,"message":"Shared health check is not enabled"}"#
    );
    assert_eq!(source.calls(), 0);
}

#[tokio::test]
async fn an_enabled_proxy_without_redis_says_so() {
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Enabled,
        status: None,
    };
    let context = caller();
    let (status, text) = route_get(
        shared_status_router(state),
        Some(&context),
        SHARED_STATUS_PATH,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text,
        r#"{"shared_health_check_enabled":true,"redis_available":false,"message":"Redis is not configured"}"#
    );
}

#[tokio::test]
async fn a_failed_status_read_is_a_500() {
    let source = FakeStatus::failing("redis is down");
    let status_source: Arc<dyn SharedHealthStatusSource> = Arc::<FakeStatus>::clone(&source);
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Enabled,
        status: Some(status_source),
    };
    let context = caller();
    let (status, text) = route_get(
        shared_status_router(state),
        Some(&context),
        SHARED_STATUS_PATH,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        text,
        r#"{"detail":{"error":"Failed to retrieve shared health check status: redis is down"}}"#
    );
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn an_unrouted_path_is_not_found() {
    let state = SharedStatusState {
        checks: SharedHealthSwitch::Enabled,
        status: None,
    };
    let context = caller();
    let (status, _text) = route_get(
        shared_status_router(state),
        Some(&context),
        "/health/shared-status/extra",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
