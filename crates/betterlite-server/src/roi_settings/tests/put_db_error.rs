#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use crate::roi_settings::auth::WRITE_FORBIDDEN;
use crate::roi_settings::error::DB_UNAVAILABLE;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::MissingRowRepository;
use crate::roi_settings::support::Reply;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::caller;
use crate::roi_settings::support::disconnected_repository;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::outage_repository;
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

fn assert_unavailable(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    let error = reply.error();
    assert_eq!(error.message, DB_UNAVAILABLE);
    assert_eq!(error.kind, "no_db_connection");
    assert_eq!(error.param.as_deref(), Some("None"));
    assert_eq!(error.code.as_deref(), Some("503"));
}

#[tokio::test]
async fn put_db_error_maps_a_database_outage_to_503() {
    let reply = call(
        settings_app(Some(Arc::new(disconnected_repository())), None),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_a_statement_timeout_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "57014",
                "canceling statement due to statement timeout",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_a_lock_timeout_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "55P03",
                "canceling statement due to lock timeout",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_an_idle_session_timeout_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "25P03",
                "terminating connection due to idle-in-transaction timeout",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_a_starting_database_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "57P03",
                "the database system is starting up",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_a_connection_exception_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "08001",
                "could not connect to server: No route to host",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_maps_a_stale_plan_to_503() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "0A000",
                "cached plan must not change result type",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_unavailable(&reply);
}

#[tokio::test]
async fn put_db_error_keeps_a_syntax_error_at_internal_server_error() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "42601",
                "syntax error at or near \"roi\"",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text(),
        r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
}

#[tokio::test]
async fn put_db_error_keeps_a_unique_violation_at_internal_server_error() {
    let reply = call(
        settings_app(
            Some(outage_repository(
                "23505",
                "duplicate key value violates unique constraint \"pkey\"",
            )),
            None,
        ),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text(),
        r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
}

#[tokio::test]
async fn put_db_error_maps_other_failures_to_internal_server_error() {
    let reply = call(
        settings_app(Some(Arc::new(MissingRowRepository)), None),
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text(),
        r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
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
