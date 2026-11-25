#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::header::{HeaderMap, HeaderValue};

use super::caller_filter::caller_headers_without_secrets;
use super::header_sets::VERTEX_NEVER_FORWARDED;
use super::support::{caller, forwarded_value, headers, jwt, secret, settings, sorted_names};
use crate::dev_auth_mode::AuthModeFlags;

#[test]
fn ordinary_headers_are_forwarded_with_lowercased_names() {
    let map = headers(&[
        ("Content-Type", "application/json"),
        ("X-Request-Id", "r-1"),
    ]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = caller_headers_without_secrets(&map, &caller(None, None), &[], &config);
    assert_eq!(
        sorted_names(&forwarded),
        vec!["content-type", "x-request-id"]
    );
}

#[test]
fn the_provider_set_and_the_mapped_route_header_are_dropped() {
    let map = headers(&[
        ("Host", "proxy"),
        ("Content-Length", "10"),
        ("Api-Key", "a"),
        ("X-Api-Key", "b"),
        ("Ocp-Apim-Subscription-Key", "c"),
        ("X-Litellm-Api-Key", "d"),
        ("Authorization", "Bearer google"),
        ("X-Goog-Api-Key", "e"),
        ("X-Custom", "v"),
        ("Litellm_User_Api_Key", "f"),
    ]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded =
        caller_headers_without_secrets(&map, &caller(None, None), &VERTEX_NEVER_FORWARDED, &config);
    assert_eq!(
        sorted_names(&forwarded),
        vec!["authorization", "x-custom", "x-goog-api-key"]
    );
}

#[test]
fn the_global_override_header_is_dropped_in_any_case() {
    let config = settings(
        None,
        AuthModeFlags::default(),
        true,
        Some("x-custom-Key"),
        &[],
    );
    let map = headers(&[("X-CUSTOM-KEY", "secret"), ("X-Safe", "v")]);
    let forwarded = caller_headers_without_secrets(&map, &caller(None, None), &[], &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-safe"]);
}

#[test]
fn an_endpoint_configured_header_is_dropped_in_any_case() {
    let mut endpoint = JsonObject::new();
    endpoint.insert(
        "litellm_user_api_key".to_owned(),
        Json::Text(String::from("X-Endpoint-Key")),
    );
    let config = settings(None, AuthModeFlags::default(), true, None, &[&endpoint]);
    let map = headers(&[("X-ENDPOINT-KEY", "secret"), ("X-Safe", "v")]);
    let forwarded = caller_headers_without_secrets(&map, &caller(None, None), &[], &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-safe"]);
}

#[test]
fn a_header_carrying_the_master_key_is_dropped() {
    let master_key = secret("mk-1");
    let config = settings(
        Some(&master_key),
        AuthModeFlags::default(),
        false,
        None,
        &[],
    );
    let map = headers(&[
        ("X-Caller-Token", "Bearer mk-1"),
        ("X-Bare-Master", "mk-1"),
        ("X-Safe", "value"),
    ]);
    let forwarded = caller_headers_without_secrets(&map, &caller(None, None), &[], &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-safe"]);
}

#[test]
fn a_header_carrying_the_stored_key_is_dropped() {
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let auth = caller(Some("stored-key"), None);
    let map = headers(&[("X-Caller-Token", "Basic stored-key"), ("X-Safe", "value")]);
    let forwarded = caller_headers_without_secrets(&map, &auth, &[], &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-safe"]);
}

#[test]
fn a_header_carrying_the_authenticated_jwt_is_dropped() {
    let stored = r#"{"sub":"user-42"}"#;
    let auth = caller(None, Some(stored));
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let authenticated_token = jwt(stored);
    let other_token = jwt(r#"{"sub":"other"}"#);
    let map = headers(&[
        ("X-Caller-Token", authenticated_token.as_str()),
        ("X-Other-Token", other_token.as_str()),
    ]);
    let forwarded = caller_headers_without_secrets(&map, &auth, &[], &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-other-token"]);
}

#[test]
fn no_auth_dev_mode_keeps_the_stored_key_value() {
    let config = settings(None, AuthModeFlags::default(), false, None, &[]);
    let auth = caller(Some("stored-key"), None);
    let map = headers(&[("X-Caller-Token", "stored-key")]);
    let forwarded = caller_headers_without_secrets(&map, &auth, &[], &config);
    assert!(forwarded_value(&forwarded, "x-caller-token") == Some("stored-key"));
}

#[test]
fn a_repeated_header_is_checked_by_its_first_value() {
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let auth = caller(Some("stored-key"), None);
    let mut map = HeaderMap::new();
    map.append("x-caller-token", HeaderValue::from_static("stored-key"));
    map.append("x-caller-token", HeaderValue::from_static("harmless"));
    let forwarded = caller_headers_without_secrets(&map, &auth, &[], &config);
    assert!(sorted_names(&forwarded).is_empty());
}

#[test]
fn a_repeated_header_keeps_the_first_value_when_it_is_harmless() {
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let auth = caller(Some("stored-key"), None);
    let mut map = HeaderMap::new();
    map.append("x-caller-token", HeaderValue::from_static("harmless"));
    map.append("x-caller-token", HeaderValue::from_static("stored-key"));
    let forwarded = caller_headers_without_secrets(&map, &auth, &[], &config);
    assert!(forwarded_value(&forwarded, "x-caller-token") == Some("harmless"));
}

#[test]
fn a_header_value_keeps_its_latin_1_text() {
    let mut map = HeaderMap::new();
    let value = HeaderValue::from_bytes(b"caf\xe9").expect("obs-text stays a header value");
    map.insert("x-note", value);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = caller_headers_without_secrets(&map, &caller(None, None), &[], &config);
    assert!(forwarded_value(&forwarded, "x-note") == Some("café"));
}

#[test]
fn no_headers_forward_nothing() {
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = caller_headers_without_secrets(
        &HeaderMap::new(),
        &caller(None, None),
        &VERTEX_NEVER_FORWARDED,
        &config,
    );
    assert!(forwarded.is_empty());
}
