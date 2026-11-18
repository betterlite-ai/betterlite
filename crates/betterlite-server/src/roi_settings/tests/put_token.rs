#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_roi::settings::ROISettingsResponse;
use secrecy::ExposeSecret as _;

use crate::roi_settings::store::SettingsCipher;
use crate::roi_settings::support::FailingCipher;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::cipher;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::sealed;
use crate::roi_settings::support::settings_app;
use crate::roi_settings::support::settings_app_with_cipher;
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
    assert_eq!(
        cipher().open(&saved.github_token).expose_secret(),
        "ghp-supplied"
    );
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
    assert_eq!(
        cipher().open(&saved.estimator_key).expose_secret(),
        "sk-supplied"
    );
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

const UNDECRYPTABLE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[tokio::test]
async fn put_token_keeps_an_unreadable_token_ciphertext_until_replaced() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = UNDECRYPTABLE.to_owned();
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_github_token);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.github_token, UNDECRYPTABLE);
}

#[tokio::test]
async fn put_token_rewrites_an_unreadable_estimator_key_as_empty() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.estimator_key = UNDECRYPTABLE.to_owned();
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"backfill_days":10}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_estimator_key);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.estimator_key, "");
}

#[tokio::test]
async fn put_token_reports_an_encryption_failure() {
    let memory = Arc::new(MemoryRepository::default());
    let app = settings_app_with_cipher(
        Some(erased(&memory)),
        Some(catalog(Vec::new())),
        Arc::new(FailingCipher),
    );
    let reply = call(
        app,
        put_request(Some(&admin_caller()), r#"{"github_token":"ghp-supplied"}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text(),
        r#"{"error":{"message":"Internal server error","type":"internal_server_error"}}"#
    );
    assert!(memory.is_empty());
}
