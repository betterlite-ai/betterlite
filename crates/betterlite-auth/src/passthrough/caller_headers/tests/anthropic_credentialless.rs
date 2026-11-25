#![cfg(test)]

use http::StatusCode;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::ANTHROPIC_MISSING_CREDENTIAL;
use super::support::{caller, forwarded_value, headers, secret, settings, sorted_names};
use super::upstream::anthropic_upstream_headers;
use crate::dev_auth_mode::AuthModeFlags;

#[test]
fn an_authorization_header_reaches_anthropic_without_the_other_credentials() {
    let map = headers(&[
        ("Authorization", "Bearer anthropic-token"),
        ("X-Goog-Api-Key", "google"),
        ("Host", "proxy"),
        ("Content-Length", "10"),
        ("Accept-Encoding", "gzip"),
        ("X-Custom", "v"),
    ]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = anthropic_upstream_headers(&map, &caller(None, None), None, &config)
        .expect("the authorization header is an Anthropic credential");
    assert_eq!(sorted_names(&forwarded), vec!["authorization", "x-custom"]);
}

#[test]
fn an_x_api_key_alone_reaches_anthropic() {
    let map = headers(&[("X-Api-Key", "sk-ant"), ("X-Goog-Api-Key", "google")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = anthropic_upstream_headers(&map, &caller(None, None), None, &config)
        .expect("the x-api-key header is an Anthropic credential");
    assert_eq!(sorted_names(&forwarded), vec!["x-api-key"]);
}

#[test]
fn no_anthropic_credential_is_a_401_with_litellms_message() {
    let map = headers(&[("X-Goog-Api-Key", "google"), ("X-Custom", "v")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let Err(error) = anthropic_upstream_headers(&map, &caller(None, None), None, &config) else {
        panic!("a request with no Anthropic credential is refused");
    };
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error.to_string(), ANTHROPIC_MISSING_CREDENTIAL);
}

#[test]
fn a_proxy_credential_supplies_the_missing_credential() {
    let mut proxy = IndexMap::new();
    proxy.insert("x-api-key".to_owned(), secret("proxy-key"));
    let map = headers(&[("X-Custom", "v")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = anthropic_upstream_headers(&map, &caller(None, None), Some(&proxy), &config)
        .expect("a configured proxy credential never 401s");
    assert_eq!(forwarded_value(&forwarded, "x-api-key"), Some("proxy-key"));
    assert_eq!(sorted_names(&forwarded), vec!["x-api-key", "x-custom"]);
}

#[test]
fn a_proxy_credential_overrides_the_callers_credential() {
    let mut proxy = IndexMap::new();
    proxy.insert("x-api-key".to_owned(), secret("proxy-key"));
    let map = headers(&[
        ("X-Api-Key", "caller-key"),
        ("Authorization", "Bearer caller-token"),
    ]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = anthropic_upstream_headers(&map, &caller(None, None), Some(&proxy), &config)
        .expect("the proxy credential is configured");
    assert_eq!(forwarded_value(&forwarded, "x-api-key"), Some("proxy-key"));
    assert_eq!(
        forwarded_value(&forwarded, "authorization"),
        Some("Bearer caller-token")
    );
}

#[test]
fn an_empty_proxy_credential_still_counts_as_configured() {
    let proxy: IndexMap<String, SecretString> = IndexMap::new();
    let map = headers(&[("X-Custom", "v")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = anthropic_upstream_headers(&map, &caller(None, None), Some(&proxy), &config)
        .expect("an empty proxy credential map is still configured");
    assert_eq!(sorted_names(&forwarded), vec!["x-custom"]);
}

#[test]
fn an_authorization_that_is_the_master_key_is_not_an_anthropic_credential() {
    let master_key = secret("mk-1");
    let config = settings(Some(&master_key), AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("Authorization", "Bearer mk-1")]);
    let Err(error) = anthropic_upstream_headers(&map, &caller(None, None), None, &config) else {
        panic!("the stripped master key leaves no Anthropic credential");
    };
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn an_x_api_key_that_is_the_stored_key_is_not_an_anthropic_credential() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("X-Api-Key", "stored-key")]);
    let Err(error) = anthropic_upstream_headers(&map, &auth, None, &config) else {
        panic!("the stripped stored key leaves no Anthropic credential");
    };
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn the_litellm_virtual_key_is_never_an_anthropic_credential() {
    let map = headers(&[("X-Litellm-Api-Key", "sk-virtual")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let Err(error) = anthropic_upstream_headers(&map, &caller(None, None), None, &config) else {
        panic!("the LiteLLM virtual key never reaches Anthropic");
    };
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn a_stripped_caller_credential_is_replaced_by_the_proxy_credential() {
    let mut proxy = IndexMap::new();
    proxy.insert("x-api-key".to_owned(), secret("proxy-key"));
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("X-Api-Key", "stored-key")]);
    let forwarded = anthropic_upstream_headers(&map, &auth, Some(&proxy), &config)
        .expect("the proxy credential is configured");
    assert_eq!(forwarded_value(&forwarded, "x-api-key"), Some("proxy-key"));
    assert_eq!(sorted_names(&forwarded), vec!["x-api-key"]);
}
