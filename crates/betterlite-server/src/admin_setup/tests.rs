#![cfg(test)]

use super::{ProxyAvailability, admin_ui_disabled, missing_keys_form, show_missing_vars_in_env};
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::Response;
use rstest::rstest;

async fn body_text(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn returns_none_when_master_key_and_database_are_both_present() {
    assert!(
        show_missing_vars_in_env(ProxyAvailability {
            has_master_key: true,
            has_database: true,
        })
        .is_none()
    );
}

#[rstest]
#[case::both_missing(false, false, "DATABASE_URL, LITELLM_MASTER_KEY")]
#[case::database_missing(true, false, "DATABASE_URL")]
#[case::master_key_missing(false, true, "LITELLM_MASTER_KEY")]
#[tokio::test]
async fn lists_the_missing_variables(
    #[case] has_master_key: bool,
    #[case] has_database: bool,
    #[case] expected_missing: &str,
) {
    let response = show_missing_vars_in_env(ProxyAvailability {
        has_master_key,
        has_database,
    })
    .expect("a variable is missing");
    let (status, html) = body_text(response).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(&format!("<p>{expected_missing}</p>")));
}

#[test]
fn missing_keys_form_substitutes_the_placeholder_once() {
    let html = missing_keys_form("DATABASE_URL, LITELLM_MASTER_KEY");
    assert!(!html.contains("@@MISSING_KEYS@@"));
    assert!(html.contains("<p>DATABASE_URL, LITELLM_MASTER_KEY</p>"));
}

#[test]
fn missing_keys_form_is_a_complete_html_page() {
    let html = missing_keys_form("DATABASE_URL");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.contains("charset=\"UTF-8\""));
    assert!(html.contains("name=\"viewport\" content=\"width=device-width, initial-scale=1.0\""));
    assert!(html.contains("body {"));
    assert!(html.contains(".container {"));
    assert!(html.contains("h1 {"));
    assert!(html.contains("pre {"));
    assert!(html.contains("<h1>Environment Setup Instructions</h1>"));
}

#[test]
fn missing_keys_form_lists_the_master_key_generator_and_database_example() {
    let html = missing_keys_form("DATABASE_URL");
    assert!(html.contains(r#"echo "LITELLM_MASTER_KEY=sk-$(openssl rand -hex 32)""#));
    assert!(html.contains(r#"LITELLM_MASTER_KEY=""#));
    assert!(html.contains(r#"DATABASE_URL="postgres://...""#));
}

#[test]
fn missing_keys_form_links_to_discord_and_docs_in_new_tabs() {
    let html = missing_keys_form("DATABASE_URL");
    assert!(html.contains(
        r#"<a href="https://discord.com/invite/wuPM9dRgDw" target="_blank">https://discord.com/invite/wuPM9dRgDw</a>"#
    ));
    assert!(html.contains(
        r#"<a href="https://docs.litellm.ai/docs/" target="_blank">https://docs.litellm.ai/docs/</a>"#
    ));
}

#[tokio::test]
async fn admin_ui_disabled_returns_a_200_html_page() {
    let (status, html) = body_text(admin_ui_disabled()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.contains("body {"));
    assert!(html.contains(".container {"));
    assert!(html.contains("<h1>Admin UI is Disabled</h1>"));
    assert!(html.contains(r#"DISABLE_ADMIN_UI="False""#));
}

#[tokio::test]
async fn admin_ui_disabled_links_to_discord_and_docs_in_new_tabs() {
    let (_, html) = body_text(admin_ui_disabled()).await;
    assert!(html.contains(
        r#"<a href="https://discord.com/invite/wuPM9dRgDw" target="_blank">https://discord.com/invite/wuPM9dRgDw</a>"#
    ));
    assert!(html.contains(
        r#"<a href="https://docs.litellm.ai/docs/" target="_blank">https://docs.litellm.ai/docs/</a>"#
    ));
}
