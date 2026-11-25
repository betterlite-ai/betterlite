#![cfg(test)]

use http::StatusCode;

use super::error::VERTEX_MISSING_CREDENTIAL;
use super::support::{caller, forwarded_value, headers, secret, settings, sorted_names};
use super::upstream::vertex_credentialless_headers;
use crate::dev_auth_mode::AuthModeFlags;

#[test]
fn an_authorization_header_reaches_google_without_the_other_credentials() {
    let map = headers(&[
        ("Authorization", "Bearer google-token"),
        ("X-Api-Key", "sk-ant"),
        ("Api-Key", "a"),
        ("Ocp-Apim-Subscription-Key", "c"),
        ("X-Litellm-Api-Key", "d"),
        ("X-Custom", "v"),
        ("Host", "proxy"),
        ("Content-Length", "10"),
        ("Accept-Encoding", "gzip"),
    ]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = vertex_credentialless_headers(&map, &caller(None, None), &config)
        .expect("the authorization header is a Google credential");
    assert_eq!(
        sorted_names(&forwarded),
        vec!["accept-encoding", "authorization", "x-custom"]
    );
}

#[test]
fn an_x_goog_api_key_alone_reaches_google() {
    let map = headers(&[("X-Goog-Api-Key", "google-key"), ("X-Api-Key", "sk-ant")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = vertex_credentialless_headers(&map, &caller(None, None), &config)
        .expect("the x-goog-api-key header is a Google credential");
    assert_eq!(sorted_names(&forwarded), vec!["x-goog-api-key"]);
}

#[test]
fn no_google_credential_is_a_401_with_litellms_message() {
    let map = headers(&[("X-Api-Key", "sk-ant"), ("X-Custom", "v")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let Err(error) = vertex_credentialless_headers(&map, &caller(None, None), &config) else {
        panic!("a request with no Google credential is refused");
    };
    assert!(error.status() == StatusCode::UNAUTHORIZED);
    assert!(error.to_string() == VERTEX_MISSING_CREDENTIAL);
}

#[test]
fn an_authorization_that_is_the_master_key_is_not_a_google_credential() {
    let master_key = secret("mk-1");
    let config = settings(Some(&master_key), AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("Authorization", "Bearer mk-1")]);
    let Err(error) = vertex_credentialless_headers(&map, &caller(None, None), &config) else {
        panic!("the stripped master key leaves no Google credential");
    };
    assert!(error.status() == StatusCode::UNAUTHORIZED);
}

#[test]
fn an_x_goog_api_key_that_is_the_master_key_is_not_a_google_credential() {
    let master_key = secret("mk-1");
    let config = settings(Some(&master_key), AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("X-Goog-Api-Key", "mk-1")]);
    let Err(error) = vertex_credentialless_headers(&map, &caller(None, None), &config) else {
        panic!("the stripped master key leaves no Google credential");
    };
    assert!(error.status() == StatusCode::UNAUTHORIZED);
}

#[test]
fn an_authorization_that_is_the_stored_key_is_not_a_google_credential() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("Authorization", "Bearer stored-key")]);
    let Err(error) = vertex_credentialless_headers(&map, &auth, &config) else {
        panic!("the stripped stored key leaves no Google credential");
    };
    assert!(error.status() == StatusCode::UNAUTHORIZED);
}

#[test]
fn the_litellm_virtual_key_is_never_a_google_credential() {
    let map = headers(&[("X-Litellm-Api-Key", "sk-virtual")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let Err(error) = vertex_credentialless_headers(&map, &caller(None, None), &config) else {
        panic!("the LiteLLM virtual key never reaches Google");
    };
    assert!(error.status() == StatusCode::UNAUTHORIZED);
}

#[test]
fn no_auth_dev_mode_keeps_a_stored_key_as_the_google_credential() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), false, None, &[]);
    let map = headers(&[("Authorization", "stored-key")]);
    let forwarded = vertex_credentialless_headers(&map, &auth, &config)
        .expect("dev mode authenticated nothing, so the value stays");
    assert!(forwarded_value(&forwarded, "authorization") == Some("stored-key"));
}
