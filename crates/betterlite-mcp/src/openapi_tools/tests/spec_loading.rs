use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use crate::openapi_tools::error::OpenApiError;
use crate::openapi_tools::spec_loading::load_openapi_spec_async;

use super::http_server::{TestServer, ok_json, status, status_with_body};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/openapi_spec");

fn corpus(name: &str) -> String {
    format!("{CORPUS}/{name}")
}

fn fetcher() -> UserUrlFetcher<SystemResolver> {
    UserUrlFetcher {
        validator: UrlValidator::new(SystemResolver, &[], &Verification::Enabled),
        validation: false,
    }
}

fn client(limits: TransportLimits) -> Arc<ProxiedClient> {
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::build(base_builder(), &policy, limits).unwrap())
}

#[tokio::test]
async fn reads_a_local_json_file() {
    let client = client(TransportLimits::DEFAULT);
    let document = load_openapi_spec_async(&fetcher(), &client, &corpus("petstore.json"), None)
        .await
        .unwrap();
    assert_eq!(
        document.servers[0].url.as_deref(),
        Some("https://petstore.test/v1")
    );
    assert!(document.paths.contains_key("/pets"));
}

#[tokio::test]
async fn reads_an_empty_document() {
    let client = client(TransportLimits::DEFAULT);
    let document = load_openapi_spec_async(&fetcher(), &client, &corpus("minimal.json"), None)
        .await
        .unwrap();
    assert!(document.paths.is_empty());
}

#[tokio::test]
async fn a_missing_local_file_names_its_location() {
    let location = format!("{CORPUS}/does-not-exist.json");
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, &location, None)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::SpecNotFound { .. }));
    assert_eq!(
        error.to_string(),
        format!("OpenAPI spec not found at {location}")
    );
}

#[tokio::test]
async fn a_non_http_location_is_read_as_a_file() {
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, "file:///elsewhere/spec.json", None)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::SpecNotFound { .. }));
}

#[tokio::test]
async fn invalid_local_json_is_a_json_error() {
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, &corpus("invalid.json"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::Json { .. }));
}

#[tokio::test]
async fn fetches_an_http_spec() {
    let body = fs::read_to_string(corpus("petstore.json")).unwrap();
    let server = TestServer::start(move |_| ok_json(&body)).await;
    let client = client(TransportLimits::DEFAULT);
    let document = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), None)
        .await
        .unwrap();
    assert_eq!(
        document.servers[0].url.as_deref(),
        Some("https://petstore.test/v1")
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_status_of_400_is_an_error() {
    let server = TestServer::start(|_| status(404)).await;
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiError::SpecStatus { status: 404, .. }
    ));
}

#[tokio::test]
async fn a_body_error_wins_over_a_status_error() {
    let server = TestServer::start(|_| status_with_body(404, r#"{"paths":{}}"#)).await;
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), Some(4))
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::SpecBody { .. }));
}

#[tokio::test]
async fn a_body_over_max_bytes_is_an_error() {
    let server = TestServer::start(|_| ok_json(r#"{"paths":{}}"#)).await;
    let client = client(TransportLimits::DEFAULT);
    let error = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), Some(4))
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::SpecBody { .. }));
}

#[tokio::test]
async fn a_body_within_max_bytes_is_read() {
    let server = TestServer::start(|_| ok_json(r#"{"paths":{}}"#)).await;
    let client = client(TransportLimits::DEFAULT);
    let document = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), Some(1024))
        .await
        .unwrap();
    assert!(document.paths.is_empty());
}

#[tokio::test]
async fn the_client_body_limit_applies_without_max_bytes() {
    let server = TestServer::start(|_| ok_json(r#"{"paths":{}}"#)).await;
    let limits = TransportLimits {
        body_bytes: 4,
        ..TransportLimits::DEFAULT
    };
    let client = client(limits);
    let error = load_openapi_spec_async(&fetcher(), &client, &server.url("/spec"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiError::SpecBody { .. }));
}
