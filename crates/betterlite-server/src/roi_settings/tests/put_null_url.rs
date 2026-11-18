#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;

use crate::roi_settings::error::INVALID_SETTINGS;
use crate::roi_settings::error::NULL_URL;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::settings_app;

#[tokio::test]
async fn put_null_url_is_rejected_with_422() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_api_url":null}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), NULL_URL);
    assert!(memory.is_empty());
}

#[tokio::test]
async fn put_null_url_is_rejected_next_to_other_fields() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"github_api_url":null,"repos":["owner/repo"]}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), NULL_URL);
    assert!(memory.is_empty());
}

#[tokio::test]
async fn put_null_url_reports_broken_stored_settings_first() {
    let memory = Arc::new(MemoryRepository::default());
    memory.seed(SETTINGS_KEY, r#"{"repos":["not-a-repository"]}"#);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_api_url":null}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail(), INVALID_SETTINGS);
}
