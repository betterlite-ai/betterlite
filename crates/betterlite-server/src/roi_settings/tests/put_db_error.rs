#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use crate::roi_settings::auth::WRITE_FORBIDDEN;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::caller;
use crate::roi_settings::support::disconnected_repository;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::settings_app;

#[tokio::test]
async fn put_db_error_without_a_database_is_db_not_connected() {
    let reply = call(
        settings_app(None, None),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn put_db_error_from_the_repository_is_an_internal_error() {
    let reply = call(
        settings_app(Some(Arc::new(disconnected_repository())), None),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail(), "Internal server error");
}

#[tokio::test]
async fn put_db_error_checks_the_write_admin_before_the_database() {
    let reply = call(
        settings_app(None, None),
        put_request(
            Some(&caller("user", vec![Role::PlatformViewer])),
            r#"{"backfill_days":10}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn put_db_error_checks_the_write_admin_before_the_body() {
    let memory = Arc::new(MemoryRepository::default());
    let reply = call(
        settings_app(Some(erased(&memory)), None),
        put_request(
            Some(&caller("user", vec![Role::PlatformViewer])),
            r#"{"backfill_days":"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn put_db_error_reports_a_missing_database_before_a_broken_body() {
    let reply = call(
        settings_app(None, None),
        put_request(Some(&admin_caller()), r#"{"backfill_days":"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}
