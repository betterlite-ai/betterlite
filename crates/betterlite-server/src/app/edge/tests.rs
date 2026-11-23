#![cfg(test)]

use std::collections::BTreeMap;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use rstest::rstest;
use tower::ServiceExt as _;

use crate::app::mount_public;
use crate::routes::tests::{empty_config, state_for};

// every docs setting starts unset so the host running the tests cannot move the routes
const FIXED_ENVIRONMENT: [(&str, Option<&str>); 7] = [
    ("DOCS_URL", None),
    ("REDOC_URL", None),
    ("OPENAPI_URL", None),
    ("NO_DOCS", None),
    ("NO_REDOC", None),
    ("NO_OPENAPI", None),
    ("ROOT_REDIRECT_URL", None),
];

#[derive(Debug, serde::Deserialize)]
struct ServedDocument {
    info: ServedInfo,
    #[serde(default)]
    servers: Vec<ServedServer>,
}

#[derive(Debug, serde::Deserialize)]
struct ServedInfo {
    title: String,
    version: String,
}

#[derive(Debug, serde::Deserialize)]
struct ServedServer {
    url: String,
}

fn config_with(lookup: &[(&str, &str)]) -> Config {
    ConfigLoader {
        lookup: lookup
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<String, String>>(),
    }
    .load("")
    .unwrap()
}

fn mounted(overrides: &[(&str, &str)], config: &Config) -> Router {
    let state = state_for(config);
    let mut variables: Vec<(&str, Option<&str>)> = FIXED_ENVIRONMENT.to_vec();
    for (name, value) in overrides {
        variables.push((*name, Some(*value)));
    }
    temp_env::with_vars(variables, || mount_public(Router::new(), &state))
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
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body.to_vec())
}

fn served_document(body: &[u8]) -> ServedDocument {
    serde_json::from_slice(body).unwrap()
}

#[rstest]
#[case("/public/providers")]
#[case("/public/providers/fields")]
#[case("/public/endpoints")]
#[tokio::test]
async fn public_provider_routes_are_served_without_a_key(#[case] path: &str) {
    let router = mounted(&[], &empty_config());
    let (status, headers, _) = send(&router, Method::GET, path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn public_swagger_page_is_served_at_the_root() {
    let router = mounted(&[], &empty_config());
    let (status, headers, body) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    let page = String::from_utf8(body).unwrap();
    assert!(page.contains("<title>LiteLLM API - Swagger UI</title>"));
    assert!(page.contains("url: '/openapi.json'"));
}

#[tokio::test]
async fn public_openapi_document_is_served_with_the_crate_version() {
    let router = mounted(&[], &empty_config());
    let (status, headers, first) = send(&router, Method::GET, "/openapi.json").await;
    let (_, _, second) = send(&router, Method::GET, "/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(first, second);
    let document = served_document(&first);
    assert_eq!(document.info.title, "LiteLLM API");
    assert_eq!(document.info.version, env!("CARGO_PKG_VERSION"));
    assert!(document.servers.is_empty());
}

#[tokio::test]
async fn public_openapi_url_moves_the_document() {
    let router = mounted(&[("OPENAPI_URL", "/spec.json")], &empty_config());
    let (status, _, _) = send(&router, Method::GET, "/openapi.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, headers, body) = send(&router, Method::GET, "/spec.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(
        served_document(&body).info.version,
        env!("CARGO_PKG_VERSION")
    );
    let (_, _, page) = send(&router, Method::GET, "/").await;
    assert!(
        String::from_utf8(page)
            .unwrap()
            .contains("url: '/spec.json'")
    );
}

#[tokio::test]
async fn public_no_openapi_removes_the_document_and_the_pages() {
    let router = mounted(&[("NO_OPENAPI", "true")], &empty_config());
    for path in ["/openapi.json", "/", "/redoc", "/docs/oauth2-redirect"] {
        let (status, _, _) = send(&router, Method::GET, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} is unmounted");
    }
}

#[tokio::test]
async fn public_merged_routes_keep_the_default_fallback() {
    let router = mounted(&[], &empty_config());
    let (status, _, body) = send(&router, Method::GET, "/definitely-not-a-route").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());
}

#[tokio::test]
async fn public_swagger_page_and_document_use_the_server_root_path() {
    let config = config_with(&[("SERVER_ROOT_PATH", "/api")]);
    let router = mounted(&[], &config);
    let (status, _, body) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("url: '/api/openapi.json'")
    );
    let (status, _, body) = send(&router, Method::GET, "/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    let document = served_document(&body);
    assert_eq!(document.servers.len(), 1);
    assert_eq!(document.servers[0].url, "/api");
}

#[tokio::test]
async fn public_docs_title_reaches_the_page_and_the_document() {
    let config = config_with(&[("DOCS_TITLE", "Acme API")]);
    let router = mounted(&[], &config);
    let (_, _, body) = send(&router, Method::GET, "/").await;
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("<title>Acme API - Swagger UI</title>")
    );
    let (_, _, body) = send(&router, Method::GET, "/openapi.json").await;
    assert_eq!(served_document(&body).info.title, "Acme API");
}

#[tokio::test]
async fn public_docs_paths_follow_the_docs_settings() {
    let router = mounted(
        &[
            ("DOCS_URL", "/docs"),
            ("REDOC_URL", "/redocly"),
            ("ROOT_REDIRECT_URL", "/ui"),
        ],
        &empty_config(),
    );
    let (status, headers, _) = send(&router, Method::GET, "/").await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/ui");
    let (status, _, _) = send(&router, Method::GET, "/docs").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&router, Method::GET, "/redoc").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&router, Method::GET, "/redocly").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn public_no_docs_and_no_redoc_remove_the_pages() {
    let router = mounted(
        &[("NO_DOCS", "true"), ("NO_REDOC", "true")],
        &empty_config(),
    );
    for path in ["/", "/redoc"] {
        let (status, _, _) = send(&router, Method::GET, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path} is unmounted");
    }
    let (status, _, _) = send(&router, Method::GET, "/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
}
