#![cfg(test)]

mod endpoints;
mod support;

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::azure_urls::{
    EnvironmentHeadersRequest, get_base_azure_url, url_parts, validate_azure_environment,
};

use support::{NoSources, credentials, load, token_settings};

#[test]
fn azure_urls_credential_identity_isolation() {
    let first = credentials("      api_key: first-identity-key\n", &[]);
    let second = credentials("      api_key: second-identity-key\n", &[]);
    assert_eq!(
        first.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("first-identity-key")
    );
    assert_eq!(
        second.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("second-identity-key")
    );
    let empty = token_settings("", &[]);
    let sources = NoSources;
    let first_headers = validate_azure_environment(EnvironmentHeadersRequest {
        headers: BTreeMap::new(),
        api_key: first.api_key.as_ref(),
        credentials: &empty,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .unwrap();
    let second_headers = validate_azure_environment(EnvironmentHeadersRequest {
        headers: BTreeMap::new(),
        api_key: second.api_key.as_ref(),
        credentials: &empty,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        first_headers.get("api-key").map(String::as_str),
        Some("first-identity-key")
    );
    assert_eq!(
        second_headers.get("api-key").map(String::as_str),
        Some("second-identity-key")
    );
    assert_eq!(first_headers.len(), 1);
    assert_eq!(second_headers.len(), 1);
}

#[test]
fn azure_urls_secret_safe_failure() {
    let settings = load("      api_key: fixture-api-key\n", &[]);
    assert!(!format!("{settings:?}").contains("fixture-api-key"));
    assert!(!format!("{:?}", settings.deployments.first()).contains("fixture-api-key"));
    let with_secret = token_settings("      client_secret: fixture-client-secret\n", &[]);
    assert!(
        !format!("{with_secret:?}").contains("fixture-client-secret"),
        "the deployment settings echoed their client secret"
    );
    let sources = NoSources;
    let error = validate_azure_environment(EnvironmentHeadersRequest {
        headers: BTreeMap::new(),
        api_key: None,
        credentials: &with_secret,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .unwrap_err();
    assert!(!format!("{error}").contains("fixture-client-secret"));
    assert!(!format!("{error:?}").contains("fixture-client-secret"));
    let invalid = get_base_azure_url(
        Some("https://user:fixture-api-key@[:::1]/openai/responses"),
        None,
        None,
        "/openai/responses",
    )
    .unwrap_err();
    assert!(!format!("{invalid}").contains("fixture-api-key"));
    assert!(!format!("{invalid:?}").contains("fixture-api-key"));
}

#[test]
fn azure_urls_url_parts_helpers() {
    assert!(url_parts::has_empty_path("https://example.com"));
    assert!(!url_parts::has_empty_path("https://example.com/"));
    assert!(!url_parts::has_empty_path("https://example.com/path"));
    assert!(url_parts::has_empty_path("https://example.com?x=1"));
    assert!(url_parts::has_empty_path("https://example.com#frag"));
    assert_eq!(
        url_parts::without_empty_path("https://example.com/"),
        "https://example.com"
    );
    assert_eq!(
        url_parts::without_empty_path("https://example.com/?x=1"),
        "https://example.com?x=1"
    );
    assert_eq!(
        url_parts::without_empty_path("https://example.com/#frag"),
        "https://example.com#frag"
    );
    assert_eq!(
        url_parts::without_empty_path("https://example.com/path"),
        "https://example.com/path"
    );
}
