#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use tower::ServiceExt as _;

use crate::routes::router;
use crate::routes::tests::{empty_config, state_for};

const REPORT_URI: &str = "/global/activity/cache_hits?start_date=2024-01-01&end_date=2024-01-31";

fn application_for(config: &Config) -> Router {
    router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    )
}

#[tokio::test]
async fn the_cache_activity_route_is_mounted_before_authentication() {
    let application = application_for(&empty_config());
    let response = application
        .oneshot(Request::get(REPORT_URI).body(Body::empty()).unwrap())
        .await
        .unwrap();
    // without a master key no caller context reaches the handler, and an unmounted path would answer 404 instead
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_route_opens_the_database_from_general_settings() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "general_settings:\n  master_key: oracle-master\n  database_url: postgresql://postgres:postgres@127.0.0.1:1/postgres\n",
    )
    .unwrap();
    let request = Request::get(REPORT_URI)
        .header(AUTHORIZATION, "Bearer oracle-master")
        .body(Body::empty())
        .unwrap();
    let response = application_for(&config).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    // the configured pool replaces the missing-database answer, and its failed query reaches the generic error body
    assert_eq!(
        &*body,
        br#"{"error":{"message":"Internal server error","type":"internal_server_error","param":null,"code":"500"}}"#
    );
}
