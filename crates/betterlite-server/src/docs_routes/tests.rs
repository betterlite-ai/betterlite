#![cfg(test)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use betterlite_config::sections::request_root_paths::Settings;
use betterlite_openapi::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};
use betterlite_openapi::docs_settings::DocsUrls;
use rstest::rstest;
use tower::ServiceExt as _;

use super::{DocsRouteConfig, docs_router};
use crate::middleware::request_root_paths::RootPaths;

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

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    host: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = host {
        builder = builder.header(header::HOST, value);
    }
    let request = builder.body(Body::empty()).unwrap();
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
    let (status, headers, body) = send(&router, Method::GET, "/", None).await;
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
    let (status, _, body) = send(&router, Method::GET, "/docs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("").into_bytes());
    let (root_status, _, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(root_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn swagger_page_prefixes_its_urls_with_the_server_root_path() {
    let router = router(default_urls(), "/api");
    let (status, _, body) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("/api").into_bytes());
}

#[tokio::test]
async fn a_trailing_slash_in_the_server_root_path_is_stripped_once() {
    let router = router(default_urls(), "/api/");
    let (status, _, body) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, expected_swagger_page("/api").into_bytes());
}

#[tokio::test]
async fn redoc_page_is_served_at_the_default_redoc_url() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/redoc", None).await;
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
    let (status, _, body) = send(&router, Method::GET, "/redoc", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        redoc_page("LiteLLM API - ReDoc", "/api/openapi.json").into_bytes()
    );
}

#[tokio::test]
async fn oauth2_redirect_page_is_served_next_to_the_swagger_page() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/docs/oauth2-redirect", None).await;
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
    let (status, headers, body) =
        send(&router, Method::GET, &format!("/swagger/{name}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, &header::CONTENT_TYPE), content_type);
    assert_eq!(body.as_slice(), swagger_asset(name).unwrap().bytes);
}

#[tokio::test]
async fn swagger_head_reports_the_length_without_the_body() {
    let router = router(default_urls(), "");
    let (status, headers, body) =
        send(&router, Method::HEAD, "/swagger/swagger-ui.css", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    let length = swagger_asset("swagger-ui.css").unwrap().bytes.len();
    assert_eq!(
        header_value(&headers, &header::CONTENT_LENGTH),
        length.to_string()
    );
}

// the documented deviation: embedded bytes have no byte-range source, so a Range request gets the full body
#[tokio::test]
async fn a_range_request_gets_the_full_body() {
    let router = router(default_urls(), "");
    let request = Request::builder()
        .uri("/swagger/swagger-ui.css")
        .header(header::RANGE, "bytes=0-9")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::CONTENT_RANGE).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        body.as_ref(),
        swagger_asset("swagger-ui.css").unwrap().bytes
    );
}

#[tokio::test]
async fn a_missing_swagger_file_is_fastapis_not_found_body() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/swagger/missing.css", None).await;
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
    let (status, headers, body) = send(&router, Method::GET, "/swagger/", None).await;
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
    let (status, _, body) = send(&router, Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_slice(),
        swagger_asset("swagger-ui.css").unwrap().bytes
    );
}

#[tokio::test]
async fn a_parent_path_that_escapes_the_mount_is_not_found() {
    let router = router(default_urls(), "");
    let (status, _, _) = send(&router, Method::GET, "/swagger/../swagger-ui.css", None).await;
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
    let (status, headers, body) =
        send(&router, Method::POST, "/swagger/swagger-ui.css", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_value(&headers, &header::ALLOW), "GET, HEAD");
    assert_eq!(body, br#"{"detail":"Method Not Allowed"}"#);
}

#[tokio::test]
async fn a_post_to_a_docs_page_is_method_not_allowed() {
    let router = router(default_urls(), "");
    let (status, headers, _) = send(&router, Method::POST, "/", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_value(&headers, &header::ALLOW), "GET, HEAD");
}

#[tokio::test]
async fn no_docs_removes_the_swagger_page_and_the_redirect_page() {
    let router = router(urls(None, Some("/redoc"), Some("/openapi.json"), None), "");
    let (swagger_status, _, _) = send(&router, Method::GET, "/", None).await;
    let (redirect_status, _, _) = send(&router, Method::GET, "/docs/oauth2-redirect", None).await;
    assert_eq!(swagger_status, StatusCode::NOT_FOUND);
    assert_eq!(redirect_status, StatusCode::NOT_FOUND);
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc", None).await;
    assert_eq!(redoc_status, StatusCode::OK);
}

#[tokio::test]
async fn no_redoc_removes_the_redoc_page() {
    let router = router(urls(Some("/"), None, Some("/openapi.json"), None), "");
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc", None).await;
    assert_eq!(redoc_status, StatusCode::NOT_FOUND);
    let (swagger_status, _, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(swagger_status, StatusCode::OK);
}

#[tokio::test]
async fn no_openapi_removes_all_three_pages() {
    let router = router(urls(Some("/"), Some("/redoc"), None, None), "");
    let (swagger_status, _, _) = send(&router, Method::GET, "/", None).await;
    let (redoc_status, _, _) = send(&router, Method::GET, "/redoc", None).await;
    let (redirect_status, _, _) = send(&router, Method::GET, "/docs/oauth2-redirect", None).await;
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
    let (root_status, headers, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(root_status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), "/ui");
    let (docs_status, _, _) = send(&router, Method::GET, "/docs", None).await;
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
    let (status, _, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn no_docs_keeps_the_root_redirect() {
    let router = router(
        urls(None, Some("/redoc"), Some("/openapi.json"), Some("/ui")),
        "",
    );
    let (status, headers, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), "/ui");
}

#[tokio::test]
async fn the_root_is_not_redirected_without_a_root_redirect_url() {
    let router = router(
        urls(Some("/docs"), Some("/redoc"), Some("/openapi.json"), None),
        "",
    );
    let (status, _, _) = send(&router, Method::GET, "/", None).await;
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
    let (status, headers, _) = send(&router, Method::GET, "/swagger", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "text/html; charset=utf-8"
    );
}

#[tokio::test]
async fn a_docs_page_on_the_swagger_directory_keeps_the_assets_served() {
    let router = router(
        urls(
            Some("/swagger/"),
            Some("/redoc"),
            Some("/openapi.json"),
            None,
        ),
        "",
    );
    let (page_status, _, _) = send(&router, Method::GET, "/swagger/", None).await;
    let (asset_status, _, body) = send(&router, Method::GET, "/swagger/swagger-ui.css", None).await;
    assert_eq!(page_status, StatusCode::OK);
    assert_eq!(asset_status, StatusCode::OK);
    assert_eq!(
        body.as_slice(),
        swagger_asset("swagger-ui.css").unwrap().bytes
    );
}

fn under_server_root_path(urls: DocsUrls, server_root_path: &str) -> Router {
    let settings = Settings {
        server_root_path: server_root_path.to_owned(),
        server_root_paths: Vec::new(),
    };
    RootPaths::from_settings(&settings)
        .unwrap()
        .wrap(router(urls, server_root_path))
}

#[tokio::test]
async fn the_docs_are_served_under_the_server_root_path() {
    let router = under_server_root_path(default_urls(), "/api");
    let (root_status, _, root_body) = send(&router, Method::GET, "/api/", None).await;
    assert_eq!(root_status, StatusCode::OK);
    assert_eq!(root_body, expected_swagger_page("/api").into_bytes());
    let (redoc_status, _, redoc_body) = send(&router, Method::GET, "/api/redoc", None).await;
    assert_eq!(redoc_status, StatusCode::OK);
    assert_eq!(
        redoc_body,
        redoc_page("LiteLLM API - ReDoc", "/api/openapi.json").into_bytes()
    );
    let (oauth_status, _, oauth_body) =
        send(&router, Method::GET, "/api/docs/oauth2-redirect", None).await;
    assert_eq!(oauth_status, StatusCode::OK);
    assert_eq!(oauth_body, oauth2_redirect_page().as_bytes());
    let (asset_status, asset_headers, asset_body) =
        send(&router, Method::GET, "/api/swagger/swagger-ui.css", None).await;
    assert_eq!(asset_status, StatusCode::OK);
    assert_eq!(
        header_value(&asset_headers, &header::CONTENT_TYPE),
        "text/css; charset=utf-8"
    );
    assert_eq!(
        asset_body.as_slice(),
        swagger_asset("swagger-ui.css").unwrap().bytes
    );
}

#[tokio::test]
async fn a_doubled_root_path_is_not_found() {
    let router = under_server_root_path(default_urls(), "/api");
    let (doubled, _, _) = send(&router, Method::GET, "/api/api/redoc", None).await;
    assert_eq!(doubled, StatusCode::NOT_FOUND);
    let trailing_slash_router = under_server_root_path(default_urls(), "/api/");
    let (trailing, _, _) = send(&trailing_slash_router, Method::GET, "/api/redoc", None).await;
    assert_eq!(trailing, StatusCode::NOT_FOUND);
}

#[rstest]
#[case::redoc("/redoc/", "http://proxy.example/redoc")]
#[case::oauth2("/docs/oauth2-redirect/", "http://proxy.example/docs/oauth2-redirect")]
#[tokio::test]
async fn a_page_path_redirects_its_extra_slash(#[case] uri: &str, #[case] location: &str) {
    let router = router(default_urls(), "");
    let (status, headers, _) = send(&router, Method::GET, uri, Some("proxy.example")).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), location);
}

#[tokio::test]
async fn a_page_path_redirects_its_extra_slash_for_any_method() {
    let router = router(default_urls(), "");
    let (status, _, _) = send(&router, Method::POST, "/redoc/", Some("proxy.example")).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn a_configured_docs_url_redirects_its_extra_slash() {
    let router = router(
        urls(Some("/docs"), Some("/redoc"), Some("/openapi.json"), None),
        "",
    );
    let (status, headers, _) = send(&router, Method::GET, "/docs/", Some("proxy.example")).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        header_value(&headers, &header::LOCATION),
        "http://proxy.example/docs"
    );
}

#[tokio::test]
async fn the_swagger_mount_redirect_keeps_the_stripped_prefix() {
    let settings = Settings {
        server_root_path: String::new(),
        server_root_paths: vec!["/tenant".to_owned()],
    };
    let root_paths = RootPaths::from_settings(&settings).unwrap();
    let router = root_paths.wrap(router(default_urls(), "/tenant"));
    let request = Request::builder()
        .uri("/tenant/swagger")
        .header(header::HOST, "proxy.example")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        header_value(response.headers(), &header::LOCATION),
        "http://proxy.example/tenant/swagger/"
    );
}

#[tokio::test]
async fn the_swagger_mount_redirect_encodes_unsafe_query_characters() {
    let router = router(default_urls(), "");
    let (status, headers, _) = send(
        &router,
        Method::GET,
        "/swagger?q=a|b",
        Some("proxy.example"),
    )
    .await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        header_value(&headers, &header::LOCATION),
        "http://proxy.example/swagger/?q=a%7Cb"
    );
}

#[rstest]
#[case::space_and_unicode("/my page/\u{fc}", "/my%20page/%C3%BC")]
#[case::pipe("/a|b", "/a%7Cb")]
#[case::percent_is_kept("/ui%2Fcatalog", "/ui%2Fcatalog")]
#[tokio::test]
async fn the_root_redirect_percent_encodes_its_location(
    #[case] configured: &str,
    #[case] location: &str,
) {
    let router = router(
        urls(
            Some("/docs"),
            Some("/redoc"),
            Some("/openapi.json"),
            Some(configured),
        ),
        "",
    );
    let (status, headers, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_value(&headers, &header::LOCATION), location);
}

#[tokio::test]
async fn the_root_redirect_is_get_only() {
    let router = router(
        urls(
            Some("/docs"),
            Some("/redoc"),
            Some("/openapi.json"),
            Some("/ui"),
        ),
        "",
    );
    let (head_status, head_headers, _) =
        send(&router, Method::HEAD, "/", Some("proxy.example")).await;
    let (post_status, post_headers, _) =
        send(&router, Method::POST, "/", Some("proxy.example")).await;
    assert_eq!(head_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(post_status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_value(&head_headers, &header::ALLOW), "GET");
    assert_eq!(header_value(&post_headers, &header::ALLOW), "GET");
}

#[tokio::test]
async fn an_invalid_utf8_swagger_path_is_not_found() {
    let router = router(default_urls(), "");
    let (status, headers, body) = send(&router, Method::GET, "/swagger/%FF", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        header_value(&headers, &header::CONTENT_TYPE),
        "application/json"
    );
    assert_eq!(body, br#"{"detail":"Not Found"}"#);
}

#[tokio::test]
async fn the_root_is_not_redirected_when_no_page_can_hold_it() {
    let router = router(urls(Some("/"), Some("/redoc"), None, Some("/ui")), "");
    let (status, _, _) = send(&router, Method::GET, "/", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
