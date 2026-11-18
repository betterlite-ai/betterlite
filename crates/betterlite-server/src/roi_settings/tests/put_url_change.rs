#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_roi::settings::ROISettingsResponse;
use secrecy::ExposeSecret as _;

use crate::roi_settings::store::SettingsCipher as _;
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
async fn put_url_change_clears_the_stored_token() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"github_api_url":"https://github.example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(!response.has_github_token);
    assert_eq!(response.github_api_url, "https://github.example.com");
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.github_token, "");
}

#[tokio::test]
async fn put_url_change_preserves_the_estimator_key() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.estimator_key = sealed("sk-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"github_api_url":"https://github.example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_estimator_key);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(
        cipher().open(&saved.estimator_key).expose_secret(),
        "sk-old"
    );
}

#[tokio::test]
async fn put_url_change_keeps_a_newly_supplied_token() {
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp-old");
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(
        app,
        put_request(
            Some(&admin_caller()),
            r#"{"github_api_url":"https://github.example.com","github_token":"ghp-new"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_github_token);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(
        cipher().open(&saved.github_token).expose_secret(),
        "ghp-new"
    );
}
