#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::caller_filter::operator_caller_key_names;
use super::support::{settings, texts};
use crate::dev_auth_mode::AuthModeFlags;

fn endpoint_headers(value: Json) -> JsonObject {
    let mut headers = JsonObject::new();
    headers.insert("litellm_user_api_key".to_owned(), value);
    headers
}

#[test]
fn no_operator_configuration_names_no_headers() {
    let config = settings(None, AuthModeFlags::default(), false, None, &[]);
    assert!(operator_caller_key_names(&config).is_empty());
}

#[test]
fn the_global_override_is_lowercased() {
    let config = settings(
        None,
        AuthModeFlags::default(),
        false,
        Some("X-Custom-Key"),
        &[],
    );
    assert_eq!(
        texts(&operator_caller_key_names(&config)),
        vec!["x-custom-key"]
    );
}

#[test]
fn a_pass_through_endpoint_names_its_caller_key_header() {
    let headers = endpoint_headers(Json::Text(String::from("X-Endpoint-Key")));
    let endpoints = [&headers];
    let config = settings(None, AuthModeFlags::default(), false, None, &endpoints);
    assert_eq!(
        texts(&operator_caller_key_names(&config)),
        vec!["x-endpoint-key"]
    );
}

#[test]
fn the_override_comes_before_the_endpoint_names() {
    let headers = endpoint_headers(Json::Text(String::from("X-Endpoint-Key")));
    let endpoints = [&headers];
    let config = settings(
        None,
        AuthModeFlags::default(),
        false,
        Some("X-Override"),
        &endpoints,
    );
    assert_eq!(
        texts(&operator_caller_key_names(&config)),
        vec!["x-override", "x-endpoint-key"]
    );
}

#[test]
fn endpoint_names_are_deduplicated_among_themselves() {
    let first = endpoint_headers(Json::Text(String::from("X-Key")));
    let second = endpoint_headers(Json::Text(String::from("x-Key")));
    let endpoints = [&first, &second];
    let config = settings(None, AuthModeFlags::default(), false, None, &endpoints);
    assert_eq!(texts(&operator_caller_key_names(&config)), vec!["x-key"]);
}

// LiteLLM's dict.fromkeys covers only the endpoint list, so the override is added even when it repeats one
#[test]
fn the_override_is_not_deduplicated_against_the_endpoint_names() {
    let headers = endpoint_headers(Json::Text(String::from("x-key")));
    let endpoints = [&headers];
    let config = settings(
        None,
        AuthModeFlags::default(),
        false,
        Some("X-Key"),
        &endpoints,
    );
    assert_eq!(
        texts(&operator_caller_key_names(&config)),
        vec!["x-key", "x-key"]
    );
}

#[test]
fn distinct_endpoint_names_keep_their_order() {
    let first = endpoint_headers(Json::Text(String::from("X-B")));
    let second = endpoint_headers(Json::Text(String::from("x-a")));
    let endpoints = [&first, &second];
    let config = settings(None, AuthModeFlags::default(), false, None, &endpoints);
    assert_eq!(
        texts(&operator_caller_key_names(&config)),
        vec!["x-b", "x-a"]
    );
}

#[test]
fn a_non_string_endpoint_value_names_no_header() {
    let headers = endpoint_headers(Json::Bool(true));
    let endpoints = [&headers];
    let config = settings(None, AuthModeFlags::default(), false, None, &endpoints);
    assert!(operator_caller_key_names(&config).is_empty());
}

#[test]
fn an_endpoint_without_the_header_key_names_no_header() {
    let headers = JsonObject::new();
    let endpoints = [&headers];
    let config = settings(None, AuthModeFlags::default(), false, None, &endpoints);
    assert!(operator_caller_key_names(&config).is_empty());
}

#[test]
fn an_empty_override_is_still_a_configured_name() {
    let config = settings(None, AuthModeFlags::default(), false, Some(""), &[]);
    assert_eq!(texts(&operator_caller_key_names(&config)), vec![""]);
}
