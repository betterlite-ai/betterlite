#![cfg(test)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use betterlite_openapi::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};
use betterlite_openapi::docs_settings::DocsUrls;
use rstest::rstest;
use tower::ServiceExt as _;

use super::{DocsRouteConfig, docs_router};

fn urls(
    docs: Option<&str>,
    redoc: Option<&str>,
    openapi: Option<&str>,
    root_redirect: Option<&str>,
) -> DocsUrls {
    DocsUrls {
        docs_url: docs.map(str::to_owned),
        redoc_url: redoc.map(str::to_owned),
        openapi_url: openapi.map(str::to_owned),
        root_redirect_url: root_redirect.map(str::to_owned),
    }
}

fn default_urls() -> DocsUrls {
    urls(Some("/"), Some("/redoc"), Some("/openapi.json"), None)
}

fn router(urls: DocsUrls, server_root_path: &str) -> Router {
    docs_router(DocsRouteConfig {
        urls,
        title: "LiteLLM API".to_owned(),
        server_root_path: server_root_path.to_owned(),
    })
}

async fn send(router: &Router, method: Method, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body)
}

fn header_value<'headers>(
    headers: &'headers HeaderMap,
    name: &header::HeaderName,
) -> &'headers str {
    headers.get(name).unwrap().to_str().unwrap()
}

fn expected_swagger_page(server_root_path: &str) -> String {
    let prefix = server_root_path.trim_end_matches('/');
    let base = if server_root_path.ends_with('/') {
        format!("{server_root_path}swagger")
    } else {
        format!("{server_root_path}/swagger")
    };
    swagger_ui_page(&SwaggerPageInputs {
        title: "LiteLLM API - Swagger UI".to_owned(),
        openapi_url: format!("{prefix}/openapi.json"),
        oauth2_redirect_url: Some(format!("{prefix}/docs/oauth2-redirect")),
        swagger_base: base,
    })
}

#[tokio::test]
async fn swagger_page_is_served_at_the_default_docs_url() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
    assert_eq!(body, expected_swagger_page("").into_bytes());
}

#[tokio::test]
async fn swagger_page_is_served_at_a_configured_docs_url() {
    let router = router(
        urls(Some("/docs"), Some("/redoc"), Some("/openapi.json"), None),
        "",
    );
    let (status, _, body) = send(&router, Method::GET, "/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("").into_bytes());
    let (root_status, _, _) = send(&router, Method::GET, "/").await;
    assert_eq!(root_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn swagger_page_prefixes_its_urls_with_the_server_root_path() {
    let router = router(default_urls(), "/api");
    let (status, _, body) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("/api").into_bytes());
}

#[tokio::test]
async fn a_trailing_slash_in_the_server_root_path_is_stripped_once() {
    let router = router(default_urls(), "/api/");
    let (status, _, body) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("/api").into_bytes());
}

#[tokio::test]
async fn redoc_page_is_served_at_the_default_redoc_url() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/redoc").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
    assert_eq!(
        body,
        redoc_page("LiteLLM API - ReDoc", "/openapi.json").into_bytes()
    );
}

#[tokio::test]
async fn redoc_page_uses_the_server_root_path() {
    let router = router(default_urls(), "/api");
    let (status, _, body) = send(&router, Method::GET, "/redoc").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        redoc_page("LiteLLM API - ReDoc", "/api/openapi.json").into_bytes()
    );
}

#[tokio::test]
async fn oauth2_redirect_page_is_served_next_to_the_swagger_page() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/docs/oauth2-redirect").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
    assert_eq!(body, oauth2_redirect_page().as_bytes());
}

#[rstest]
#[case::javascript("swagger-ui-bundle.js", "text/javascript; charset=utf-8")]
#[case::stylesheet("swagger-ui.css", "text/css; charset=utf-8")]
#[case::png("favicon.png", "image/png")]
#[case::ico("favicon.ico", "image/vnd.microsoft.icon")]
#[tokio::test]
async fn every_swagger_asset_is_served_with_its_content_type(
    #[case] name: &str,
    #[case] content_type: &str,
) {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, &format!("/swagger/{name}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, &header::CONTENT_TYPE), content_type);
    assert_eq!(body.as_slice(), swagger_asset(name).unwrap().bytes);
}

#[tokio::test]
async fn swagger_head_reports_the_length_without_the_body() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::HEAD, "/swagger/swagger-ui.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    let length = swagger_asset("swagger-ui.css").unwrap().bytes.len();
    assert_eq!(
        header_value(&headers, &header::CONTENT_LENGTH),
        length.to_string()
    );
}

#[tokio::test]
async fn a_missing_swagger_file_is_fastapis_not_found_body() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/swagger/missing.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "application/json"
    );
    assert_eq!(body, br#"{"detail":"Not Found"}"#);
}

#[tokio::test]
async fn the_swagger_directory_itself_is_not_found() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/swagger/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "application/json"
    );
    assert_eq!(body, br#"{"detail":"Not Found"}"#);
}

#[rstest]
#[case::dotdot_stays_inside("/swagger/x/../swagger-ui.css")]
#[case::trailing_slash("/swagger/swagger-ui.css/")]
#[tokio::test]
async fn a_swagger_path_is_normalized_before_the_lookup(#[case] uri: &str) {
    let router = router(default_urls(), "");
    let (status, _, body) = send(&router, Method::GET, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_slice(),
        swagger_asset("swagger-ui.css").unwrap().bytes
    );
}

#[tokio::test]
async fn a_parent_path_that_escapes_the_mount_is_not_found() {
    let router = router(default_urls(), "");
    let (status, _, _) = send(&router, Method::GET, "/swagger/../swagger-ui.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_swagger_mount_redirects_to_its_slash() {
    let router = router(default_urls(), "");
    let request = Request::builder()
        .uri("/swagger?probe=1")
        .header(header::HOST, "proxy.example:8080")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        header_value(response.headers(), &header::LOCATION),
        "http://proxy.example:8080/swagger/?probe=1"
    );
}

#[tokio::test]
async fn a_post_to_a_swagger_file_is_method_not_allowed() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::POST, "/swagger/swagger-ui.css").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_value(&headers, &header::ALLOW), "GET, HEAD");
    assert_eq!(body, br#"{"detail":"Method Not Allowed"}"#);
}

#[tokio::test]
async fn a_post_to_a_docs_page_is_method_not_allowed() {
    let router = router(default_urls(), "");
    let (status, headers, _) = send(&router, Method::POST, "/").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_value(&headers, &header::ALLOW), "GET, HEAD");
}

#[tokio::test]
async fn no_docs_removes_the_swagger_page_and_the_redirect_page() {
    let router = router(urls(None, Some("/redoc"), Some("/openapi.json"), None), "");
    let (swagger_status, _, _) = send(&router, Method::GET, "/").await;
    let (redirect_status, _, _) = send(&router, Method::GET, "/docs/oauth2-redirect").await;
    assert_eq!(swagger_status, StatusCode::NOT_FOUND);
    assert_eq!(redirect_status, StatusCode::NOT_FOUND);
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc").await;
    assert_eq!(redoc_status, StatusCode::OK);
}

#[tokio::test]
async fn no_redoc_removes_the_redoc_page() {
    let router = router(urls(Some("/"), None, Some("/openapi.json"), None), "");
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc").await;
    assert_eq!(redoc_status, StatusCode::NOT_FOUND);
    let (swagger_status, _, _) = send(&router, Method::GET, "/").await;
    assert_eq!(swagger_status, StatusCode::OK);
}

#[tokio::test]
async fn no_openapi_removes_all_three_pages() {
    let router = router(urls(Some("/"), Some("/redoc"), None, None), "");
    let (swagger_status, _, _) = send(&router, Method::GET, "/").await;
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc").await;
    let (redirect_status, _, _) = send(&router, Method::GET, "/docs/oauth2-redirect").await;
    assert_eq!(swagger_status, StatusCode::NOT_FOUND);
    assert_eq!(redoc_status, StatusCode::NOT_FOUND);
    assert_eq!(redirect_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_custom_docs_url_answers_the_root_with_the_redirect() {
    let router = router(
        urls(
            Some("/docs"),
            Some("/redoc"),
            Some("/openapi.json"),
            Some("/ui"),
        ),
        "",
    );
    let (root_status, headers, _) = send(&router, Method::GET, "/").await;
    assert_eq!(root_status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), "/ui");
    let (docs_status, _, _) = send(&router, Method::GET, "/docs").await;
    assert_eq!(docs_status, StatusCode::OK);
}

#[tokio::test]
async fn the_root_is_not_redirected_when_docs_live_at_the_root() {
    let router = router(
        urls(
            Some("/"),
            Some("/redoc"),
            Some("/openapi.json"),
            Some("/ui"),
        ),
        "",
    );
    let (status, _, _) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn no_docs_keeps_the_root_redirect() {
    let router = router(
        urls(None, Some("/redoc"), Some("/openapi.json"), Some("/ui")),
        "",
    );
    let (status, headers, _) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), "/ui");
}

#[tokio::test]
async fn the_root_is_not_redirected_without_a_root_redirect_url() {
    let router = router(
        urls(Some("/docs"), Some("/redoc"), Some("/openapi.json"), None),
        "",
    );
    let (status, _, _) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_swagger_page_takes_the_swagger_mount_path_when_configured_there() {
    let router = router(
        urls(
            Some("/swagger"),
            Some("/redoc"),
            Some("/openapi.json"),
            None,
        ),
        "",
    );
    let (status, headers, _) = send(&router, Method::GET, "/swagger").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
}
