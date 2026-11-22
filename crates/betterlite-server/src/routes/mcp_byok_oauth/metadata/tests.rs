#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{Request, StatusCode};
use axum::response::Response;

use super::super::tests::{Built, FakeCodes, FakeSink, TestApp, get, header, send, text};
use super::super::{ByokOauthRoutes, RequestBaseUrl, routes};

struct HeaderBaseUrl;

impl RequestBaseUrl for HeaderBaseUrl {
    fn base_url(&self, parts: &Parts) -> String {
        parts
            .headers
            .get("x-forwarded-base")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }
}

async fn fetch(app: TestApp, uri: &str) -> Response {
    let Built { router, .. } = app.build();
    send(router, get(uri)).await
}

#[tokio::test]
async fn the_bare_discovery_path_answers_the_authorization_server_document() {
    let response = fetch(
        TestApp::with_record(),
        "/.well-known/oauth-authorization-server",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), Some("application/json"));
    assert_eq!(header(&response, "cache-control"), None);
    assert_eq!(header(&response, "pragma"), None);
    assert_eq!(
        text(response).await,
        concat!(
            r#"{"issuer":"https://proxy.test","#,
            r#""authorization_endpoint":"https://proxy.test/v1/mcp/oauth/authorize","#,
            r#""token_endpoint":"https://proxy.test/v1/mcp/oauth/token","#,
            r#""response_types_supported":["code"],"#,
            r#""grant_types_supported":["authorization_code"],"#,
            r#""code_challenge_methods_supported":["S256"]}"#,
        )
    );
}

#[tokio::test]
async fn the_suffix_path_answers_the_oauth_issuer_document() {
    let mut app = TestApp::with_record();
    app.base_url = "https://proxy.test/root".to_owned();
    app.suffix = "/prem".to_owned();
    let response = fetch(
        app,
        "/.well-known/oauth-authorization-server/prem/v1/mcp/oauth",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains(r#""issuer":"https://proxy.test/root/v1/mcp/oauth""#));
    assert!(
        body.contains(
            r#""authorization_endpoint":"https://proxy.test/root/v1/mcp/oauth/authorize""#
        )
    );
    assert!(body.contains(r#""token_endpoint":"https://proxy.test/root/v1/mcp/oauth/token""#));
}

#[tokio::test]
async fn a_root_prefix_rebuilds_the_base_from_scheme_and_netloc() {
    let mut app = TestApp::with_record();
    app.base_url = "https://proxy.test/root".to_owned();
    app.root_prefixes = vec!["/prem".to_owned()];
    let response = fetch(
        app,
        "/.well-known/oauth-authorization-server/prem/v1/mcp/oauth",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains(r#""issuer":"https://proxy.test/prem/v1/mcp/oauth""#));
    assert!(
        body.contains(
            r#""authorization_endpoint":"https://proxy.test/prem/v1/mcp/oauth/authorize""#
        )
    );
    assert!(!body.contains("/root"));
}

#[tokio::test]
async fn the_protected_resource_path_drops_the_base_path() {
    let mut app = TestApp::with_record();
    app.base_url = "https://proxy.test/root".to_owned();
    let response = fetch(app, "/v1/mcp/oauth/protected-resource").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), None);
    assert_eq!(
        text(response).await,
        concat!(
            r#"{"resource":"https://proxy.test","#,
            r#""authorization_servers":["https://proxy.test/root/v1/mcp/oauth"]}"#,
        )
    );
}

#[tokio::test]
async fn an_unparsable_base_url_answers_500_on_the_protected_resource_path() {
    let mut app = TestApp::with_record();
    app.base_url = "http://[::1".to_owned();
    let response = fetch(app, "/v1/mcp/oauth/protected-resource").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(text(response).await.contains(r#""code":"500""#));
}

#[tokio::test]
async fn an_unparsable_base_url_answers_500_on_a_root_prefix_path() {
    let mut app = TestApp::with_record();
    app.base_url = "http://[::1".to_owned();
    app.root_prefixes = vec!["/prem".to_owned()];
    let response = fetch(
        app,
        "/.well-known/oauth-authorization-server/prem/v1/mcp/oauth",
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(text(response).await.contains(r#""code":"500""#));
}

#[tokio::test]
async fn metadata_handlers_read_the_request_parts() {
    let router = routes(
        Arc::new(ByokOauthRoutes {
            codes: FakeCodes::new(None),
            credentials: None::<FakeSink>,
            master_key: None,
            base_url: HeaderBaseUrl,
            root_prefixes: Vec::new(),
        }),
        "",
    );
    let request = Request::builder()
        .uri("/.well-known/oauth-authorization-server")
        .header("x-forwarded-base", "https://parts.test")
        .body(Body::empty())
        .unwrap();
    let response = send(router, request).await;
    assert!(
        text(response)
            .await
            .contains(r#""issuer":"https://parts.test""#)
    );
}
