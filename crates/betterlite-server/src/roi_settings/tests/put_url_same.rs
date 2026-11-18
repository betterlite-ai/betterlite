#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_roi::settings::ROISettingsResponse;
use rstest::rstest;

use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::admin_caller;
use crate::roi_settings::support::call;
use crate::roi_settings::support::catalog;
use crate::roi_settings::support::erased;
use crate::roi_settings::support::put_request;
use crate::roi_settings::support::sealed;
use crate::roi_settings::support::settings_app;
use crate::roi_settings::support::stored_settings;

#[rstest]
#[case::same_url(
    "https://api.github.com",
    r#"{"github_api_url":"https://api.github.com"}"#
)]
#[case::trailing_slash(
    "https://api.github.com",
    r#"{"github_api_url":"https://api.github.com/"}"#
)]
#[case::stored_trailing_slash(
    "https://api.github.com/",
    r#"{"github_api_url":"https://api.github.com"}"#
)]
#[case::no_url("https://api.github.com", r#"{"backfill_days":10}"#)]
#[tokio::test]
async fn put_url_same_preserves_the_stored_ciphertext(
    #[case] current_url: &str,
    #[case] body: &str,
) {
    let token = sealed("ghp-kept");
    let memory = Arc::new(MemoryRepository::default());
    let mut stored = stored_settings();
    stored.github_api_url = current_url.to_owned();
    stored.github_token = token.clone();
    memory.seed_settings(&stored);
    let app = settings_app(Some(erased(&memory)), Some(catalog(Vec::new())));
    let reply = call(app, put_request(Some(&admin_caller()), body)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.has_github_token);
    let saved = memory.stored().expect("saved settings");
    assert_eq!(saved.github_token, token);
}
