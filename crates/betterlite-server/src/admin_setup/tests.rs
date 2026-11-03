#![cfg(test)]

use super::{ProxyAvailability, admin_ui_disabled, show_missing_vars_in_env};
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::Response;
use rstest::rstest;

const MISSING_KEYS_FIXTURE: &str = include_str!("fixtures/litellm_missing_keys_form.html");
const ADMIN_UI_DISABLED_FIXTURE: &str = include_str!("fixtures/litellm_admin_ui_disabled.html");

async fn response_parts(response: Response) -> (StatusCode, String, String) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("HTML response has content type")
        .to_str()
        .expect("content type is text");
    let content_type = content_type.to_owned();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn missing_keys_page(names: &str) -> String {
    let rendered = MISSING_KEYS_FIXTURE
        .replace("{{", "{")
        .replace("}}", "}")
        .replace("{missing_keys}", names);
    format!("\n{rendered}    ")
}

fn disabled_page() -> String {
    let rendered = ADMIN_UI_DISABLED_FIXTURE
        .replace("{{", "{")
        .replace("}}", "}");
    format!("\n{rendered}    ")
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
async fn missing_vars_response_matches_litellm(
    #[case] has_master_key: bool,
    #[case] has_database: bool,
    #[case] expected_missing: &str,
) {
    let response = show_missing_vars_in_env(ProxyAvailability {
        has_master_key,
        has_database,
    })
    .expect("a variable is missing");
    let (status, content_type, html) = response_parts(response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/html; charset=utf-8");
    assert_eq!(html, missing_keys_page(expected_missing));
}

#[tokio::test]
async fn admin_ui_disabled_matches_litellm_with_single_braces() {
    let (status, content_type, html) = response_parts(admin_ui_disabled()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/html; charset=utf-8");
    assert_eq!(html, disabled_page());
}
