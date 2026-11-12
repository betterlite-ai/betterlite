#![cfg(test)]

mod endpoints;
mod support;

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::azure_urls::{
    AzureUrlError, EnvironmentHeadersRequest, get_base_azure_url, url_parts,
    validate_azure_environment,
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
    // a complete client-secret identity makes the source lookup fail, so the error carries the credential
    let with_secret = token_settings(
        "      tenant_id: fixture-tenant\n      client_id: fixture-client\n      client_secret: fixture-client-secret\n",
        &[],
    );
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
fn azure_urls_api_base_edges() {
    assert_eq!(
        get_base_azure_url(Some("/openai/responses"), None, None, "/openai/responses").unwrap(),
        "/openai/responses"
    );
    let relative = get_base_azure_url(Some("/v1"), None, None, "/openai/responses").unwrap_err();
    assert!(matches!(relative, AzureUrlError::Compose(_)));
    assert_eq!(
        get_base_azure_url(
            Some("https://example.com/a\\b"),
            None,
            None,
            "/openai/responses"
        )
        .unwrap(),
        "https://example.com/a/b/openai/responses"
    );
}

#[test]
fn azure_urls_url_parts_helpers() {
    assert_eq!(
        url_parts::first_query_values(Some("a=1&b=2&a=3")),
        vec![
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned())
        ]
    );
    assert_eq!(
        url_parts::first_query_values(Some("a=&a=2")),
        vec![("a".to_owned(), String::new())]
    );
    assert_eq!(
        url_parts::query_pairs(Some("a=1&a=2&b")),
        vec![
            ("a".to_owned(), "1".to_owned()),
            ("a".to_owned(), "2".to_owned()),
            ("b".to_owned(), String::new())
        ]
    );
    assert_eq!(url_parts::query_pairs(None), Vec::new());
    assert_eq!(
        url_parts::encoded_query(&[("q".to_owned(), "a b*~".to_owned())]),
        Some("q=a+b%2A~".to_owned())
    );
    assert_eq!(url_parts::encoded_query(&[]), None);
}
