#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_roi::settings::ROISettingsResponse;

use crate::roi_settings::store::decrypt;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::cipher;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::sealed;
use crate::roi_settings::support::settings_app;
use crate::roi_settings::support::stored_settings;

#[tokio::test]
async fn put_token_encrypts_a_supplied_token() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_token":"ghp-supplied"}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_github_token);
    assert!(!reply.text().contains("ghp-supplied"));
    let saved = memory.stored().expect("saved settings");
    assert_ne!(saved.github_token, "ghp-supplied");
    assert_eq!(decrypt(&cipher(), &saved.github_token), "ghp-supplied");
}

#[tokio::test]
async fn put_token_stores_nothing_for_a_null_token() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_token":null}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_github_token);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.github_token, "");
}

#[tokio::test]
async fn put_token_stores_nothing_for_an_empty_token() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_token":""}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_github_token);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.github_token, "");
}

#[tokio::test]
async fn put_token_encrypts_a_supplied_estimator_key() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"estimator_key":"sk-supplied"}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_estimator_key);
    assert!(!reply.text().contains("sk-supplied"));
    let saved = memory.stored().expect("saved settings");
    assert_ne!(saved.estimator_key, "sk-supplied");
    assert_eq!(decrypt(&cipher(), &saved.estimator_key), "sk-supplied");
}

#[tokio::test]
async fn put_token_stores_nothing_for_a_null_estimator_key() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.estimator_key = sealed("sk-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"estimator_key":null}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_estimator_key);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.estimator_key, "");
}
