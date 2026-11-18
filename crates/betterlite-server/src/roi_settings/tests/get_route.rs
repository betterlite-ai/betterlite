#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_roi::settings::ROISettingsResponse;

use crate::roi_settings::auth::READ_FORBIDDEN;
use crate::roi_settings::error::DB_UNAVAILABLE;
use crate::roi_settings::error::INVALID_SETTINGS;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::caller;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::disconnected_repository;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::get_request;
use crate::roi_settings::support::sealed;
use crate::roi_settings::support::settings_app;
use crate::roi_settings::support::stored_settings;

#[tokio::test]
async fn get_route_returns_the_public_settings() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.repos = vec!["owner/repo".to_owned()];
    stored.estimator_model = "gpt-4o".to_owned();
    memory.seed_settings(&stored);
    let app = settings_app(
        Some(erased(&memory)),
        Some(catalog(vec!["gpt-4o".to_owned()])),
    );
    let reply = call(
        app,
        get_request(Some(&caller("user", vec![Role::PlatformViewer]))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.ready);
    assert_eq!(response.available_models, ["gpt-4o"]);
    assert_eq!(response.estimator_model, "gpt-4o");
    assert!(!response.has_github_token);
    assert!(!response.has_estimator_key);
}

#[tokio::test]
async fn get_route_never_returns_the_stored_secrets() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp_route_secret");
    stored.estimator_key = sealed("sk_route_secret");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(app, get_request(Some(&admin_caller()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_github_token);
    assert!(response.has_estimator_key);
    let text = reply.text();
    assert!(!text.contains("ghp_route_secret"));
    assert!(!text.contains("sk_route_secret"));
}

#[tokio::test]
async fn get_route_rejects_a_non_admin_caller() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        get_request(Some(&caller("user", vec![Role::OrgAdmin]))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), READ_FORBIDDEN);
}

#[tokio::test]
async fn get_route_reports_a_missing_database() {
    let app = settings_app(None, Some(catalog(Vec::new())));
    let reply = call(app, get_request(Some(&admin_caller()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn get_route_reports_a_database_outage() {
    let app = settings_app(
        Some(Arc::new(disconnected_repository())),
        Some(catalog(Vec::new())),
    );
    let reply = call(app, get_request(Some(&admin_caller()))).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    let error = reply.error();
    assert_eq!(error.message, DB_UNAVAILABLE);
    assert_eq!(error.kind, "no_db_connection");
    assert_eq!(error.param.as_deref(), Some("None"));
    assert_eq!(error.code.as_deref(), Some("503"));
}

#[tokio::test]
async fn get_route_reports_stored_settings_that_do_not_validate() {
    let memory = Arc::new(MemoryRepository::default());
    memory.seed(SETTINGS_KEY, r#"{"repos":["not-a-repository"]}"#);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(app, get_request(Some(&admin_caller()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail(), INVALID_SETTINGS);
}
