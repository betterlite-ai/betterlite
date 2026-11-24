#![cfg(test)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_config::sections::login_form::Settings;
use tower::ServiceExt as _;

use crate::invitation_routes::{LoginPageState, public_routes};

fn login_router(settings: &Settings) -> Router {
    public_routes(LoginPageState::from_settings(settings))
}

async fn fetch(router: &Router) -> (StatusCode, String, String) {
    let request = Request::builder()
        .method("GET")
        .uri("/fallback/login")
        .body(Body::empty())
        .expect("a request");
    let response = router.clone().oneshot(request).await.expect("a response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    (status, content_type, text)
}

#[tokio::test]
async fn fallback_login_renders_the_form_without_the_banner() {
    let (status, content_type, html) = fetch(&login_router(&Settings::default())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/html; charset=utf-8");
    assert!(html.contains("Default Credentials"));
    assert!(!html.contains(r#"<div class="deprecation-banner">"#));
}

#[tokio::test]
async fn fallback_login_keeps_the_hint_hidden_when_the_settings_hide_it() {
    let mut login = LoginPageState::from_settings(&Settings::default());
    login.hide_default_credentials_hint = true;
    let (status, _, html) = fetch(&public_routes(login)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("Default Credentials"));
    assert!(!html.contains(r#"<div class="deprecation-banner">"#));
}
