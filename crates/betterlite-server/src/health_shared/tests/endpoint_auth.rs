#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::route::{
    SHARED_STATUS_PATH, SharedHealthSwitch, SharedStatusState, shared_status_router,
};
use super::status_view::SharedHealthStatusSource;
use super::support::{FakeStatus, caller, field, route_get};

const UNAUTHORIZED_BODY: &str = r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#;

fn body_of(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the response body is JSON")
}

#[tokio::test]
async fn an_unauthenticated_call_is_rejected() {
    let source = FakeStatus::answering(JsonObject::new());
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
    let mut payload = JsonObject::new();
    payload.insert(
        "pod_id".to_owned(),
        Json::Text("pod_1704067200000".to_owned()),
    );
    payload.insert("redis_available".to_owned(), Json::Bool(true));
    let source = FakeStatus::answering(payload.clone());
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
    let body = body_of(&text);
    assert_eq!(
        field(&body, "shared_health_check_enabled"),
        Json::Bool(true)
    );
    assert_eq!(field(&body, "status"), Json::Object(payload));
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn the_disabled_proxy_says_so_and_never_reads_redis() {
    let source = FakeStatus::answering(JsonObject::new());
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
    let body = body_of(&text);
    assert_eq!(
        field(&body, "shared_health_check_enabled"),
        Json::Bool(false)
    );
    assert_eq!(
        field(&body, "message"),
        Json::Text("Shared health check is not enabled".to_owned())
    );
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
