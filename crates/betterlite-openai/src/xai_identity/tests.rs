#![cfg(test)]

use std::path::PathBuf;
use std::{fs, iter};

use betterlite_config::sections::xai_identity::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::XaiIdentityError;
use super::{
    KeyOrder, decode_xai_models, get_api_base, get_api_key, get_base_model, models_request,
    validate_environment,
};
use crate::openai_base_config::Headers;

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(xai: Option<&str>, generic: Option<&str>, environment: Option<&str>) -> Settings {
    Settings {
        xai_key: xai.map(secret),
        generic_api_key: generic.map(secret),
        environment_key: environment.map(secret),
        api_base: None,
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::explicit_wins(Some("explicit"), Some("module"), KeyOrder::Chat, Some("explicit"))]
#[case::explicit_wins_legacy(
    Some("explicit"),
    Some("module"),
    KeyOrder::GenericBeforeEnvironment,
    Some("explicit")
)]
#[case::module_key_before_environment(None, Some("module"), KeyOrder::Chat, Some("module"))]
#[case::module_key_before_generic(
    None,
    Some("module"),
    KeyOrder::GenericBeforeEnvironment,
    Some("module")
)]
#[case::chat_skips_generic(None, None, KeyOrder::Chat, Some("environment"))]
#[case::legacy_inserts_generic(None, None, KeyOrder::GenericBeforeEnvironment, Some("generic"))]
fn xai_identity_key_precedence_orders_every_source(
    #[case] explicit: Option<&str>,
    #[case] xai: Option<&str>,
    #[case] order: KeyOrder,
    #[case] expected: Option<&str>,
) {
    let settings = settings(xai, Some("generic"), Some("environment"));
    let found = get_api_key(explicit.map(secret).as_ref(), order, &settings);
    assert_eq!(found.as_ref().map(ExposeSecret::expose_secret), expected);
}

#[rstest]
#[case::chat(KeyOrder::Chat)]
#[case::legacy(KeyOrder::GenericBeforeEnvironment)]
fn xai_identity_key_precedence_falls_back_to_nothing(#[case] order: KeyOrder) {
    assert!(get_api_key(None, order, &Settings::default()).is_none());
}

#[test]
fn xai_identity_key_precedence_uses_an_explicit_empty_key_as_given() {
    let settings = settings(Some("module"), None, None);
    let key = get_api_key(Some(&secret("")), KeyOrder::Chat, &settings).unwrap();
    assert_eq!(key.expose_secret(), "");
}

#[rstest]
#[case::explicit(
    Some("https://explicit.example"),
    Some("https://env.example"),
    "https://explicit.example"
)]
#[case::environment(None, Some("https://env.example"), "https://env.example")]
#[case::default(None, None, "https://api.x.ai")]
fn xai_identity_api_base_prefers_explicit_then_environment_then_default(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let settings = Settings {
        xai_key: None,
        generic_api_key: None,
        environment_key: None,
        api_base: environment.map(str::to_owned),
    };
    assert_eq!(get_api_base(explicit, &settings), expected);
}

#[rstest]
#[case::single_prefix("xai/grok-4", "grok-4")]
#[case::repeated_prefix("xai/xai/grok-4", "grok-4")]
#[case::embedded_prefix("a/xai/b", "a/b")]
#[case::none("grok-4", "grok-4")]
fn xai_identity_base_model_removes_every_provider_prefix(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(get_base_model(model), expected);
}

#[rstest]
#[case::key_replaces_authorization(
    &[("Authorization", "Bearer old")],
    Some("fresh"),
    &[("Authorization", "Bearer fresh"), ("Content-Type", "application/json")]
)]
#[case::no_key_keeps_authorization(
    &[("Authorization", "Bearer old")],
    None,
    &[("Authorization", "Bearer old"), ("Content-Type", "application/json")]
)]
#[case::lowercase_content_type_is_kept(
    &[("content-type", "text/plain")],
    None,
    &[("content-type", "text/plain")]
)]
#[case::capitalised_content_type_is_kept(
    &[("Content-Type", "text/plain")],
    None,
    &[("Content-Type", "text/plain")]
)]
#[case::other_casing_gets_a_second_header(
    &[("CONTENT-TYPE", "text/plain")],
    None,
    &[("CONTENT-TYPE", "text/plain"), ("Content-Type", "application/json")]
)]
#[case::lowercase_authorization_stays_beside_the_new_one(
    &[("authorization", "Bearer old")],
    Some("fresh"),
    &[("Authorization", "Bearer fresh"), ("Content-Type", "application/json"), ("authorization", "Bearer old")]
)]
fn xai_identity_validate_environment_sets_headers(
    #[case] given: &[(&str, &str)],
    #[case] key: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let result = validate_environment(headers(given), key.map(secret).as_ref());
    assert_eq!(result, headers(expected));
}

#[rstest]
#[case::default_base(None, "https://api.x.ai/v1/models")]
#[case::explicit_base(Some("https://proxy.example"), "https://proxy.example/v1/models")]
#[case::base_with_v1(Some("https://proxy.example/v1"), "https://proxy.example/v1/v1/models")]
#[case::trailing_slash(Some("https://proxy.example/"), "https://proxy.example//v1/models")]
fn xai_identity_models_request_appends_the_path_literally(
    #[case] base: Option<&str>,
    #[case] expected: &str,
) {
    let settings = settings(Some("module"), None, None);
    let request = models_request(None, base, &settings).unwrap();
    assert_eq!(request.url, expected);
    assert_eq!(request.authorization.expose_secret(), "Bearer module");
}

#[test]
fn xai_identity_models_request_without_a_key_names_the_missing_credentials() {
    let error = models_request(None, None, &Settings::default()).unwrap_err();
    assert!(matches!(error, XaiIdentityError::MissingCredentials));
}

#[test]
fn xai_identity_models_request_ignores_the_generic_key() {
    let settings = settings(None, Some("generic"), None);
    let error = models_request(None, None, &settings).unwrap_err();
    assert!(matches!(error, XaiIdentityError::MissingCredentials));
}

#[test]
fn xai_identity_models_request_keeps_the_key_out_of_debug_and_errors() {
    let settings = settings(Some("sk-hidden"), None, None);
    let request = models_request(None, None, &settings).unwrap();
    assert!(!format!("{request:?}").contains("sk-hidden"));
    let error = decode_xai_models(401, b"denied").unwrap_err();
    assert!(!error.to_string().contains("sk-hidden"));
}

#[rstest]
#[case::two_models(br#"{"data":[{"id":"grok-4"},{"id":"grok-3-mini"}]}"#, &["xai/grok-4", "xai/grok-3-mini"])]
#[case::empty_list(br#"{"data":[]}"#, &[])]
#[case::already_prefixed(br#"{"data":[{"id":"xai/grok-4"}]}"#, &["xai/xai/grok-4"])]
#[case::extra_fields_ignored(br#"{"object":"list","data":[{"id":"grok-4","owned_by":"xai"}]}"#, &["xai/grok-4"])]
fn xai_identity_decode_models_prefixes_every_id(#[case] body: &[u8], #[case] expected: &[&str]) {
    assert_eq!(decode_xai_models(200, body).unwrap(), expected);
}

#[rstest]
#[case::missing_data(b"{}")]
#[case::null_data(br#"{"data":null}"#)]
#[case::empty_object_data(br#"{"data":{}}"#)]
#[case::empty_string_data(br#"{"data":""}"#)]
#[case::missing_id(br#"{"data":[{"name":"grok-4"}]}"#)]
#[case::numeric_id(br#"{"data":[{"id":4}]}"#)]
#[case::null_id(br#"{"data":[{"id":null}]}"#)]
#[case::not_json(b"<html>")]
#[case::top_level_array(b"[]")]
fn xai_identity_decode_models_rejects_malformed_catalogs(#[case] body: &[u8]) {
    let error = decode_xai_models(200, body).unwrap_err();
    assert!(matches!(error, XaiIdentityError::Decode(_)));
}

#[test]
fn xai_identity_decode_models_reports_the_status_and_body() {
    let error = decode_xai_models(503, b"overloaded").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to fetch models from XAI. Status code: 503, Response: overloaded"
    );
}

#[test]
fn xai_identity_decode_models_reports_a_non_utf8_error_body_by_length() {
    let error = decode_xai_models(502, &[0xff, 0xfe, 0xfd]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to fetch models from XAI. Status code: 502, Response: 3 bytes that are not UTF-8"
    );
}

#[rstest]
#[case::redirect(302)]
#[case::client(404)]
#[case::informational(100)]
fn xai_identity_decode_models_rejects_every_non_success_status(#[case] status: u16) {
    let error = decode_xai_models(status, br#"{"data":[]}"#).unwrap_err();
    assert!(matches!(error, XaiIdentityError::Status { .. }));
}

#[test]
fn xai_identity_decode_models_rejects_an_oversized_body() {
    let body = vec![b' '; 4 * 1024 * 1024 + 1];
    let error = decode_xai_models(200, &body).unwrap_err();
    assert!(matches!(error, XaiIdentityError::ResponseTooLarge { .. }));
}

#[test]
fn xai_identity_decode_models_rejects_deeply_nested_input() {
    let mut body = br#"{"data":"#.to_vec();
    body.extend(iter::repeat_n(b'[', 100_000));
    decode_xai_models(200, &body).unwrap_err();
}

#[rstest]
fn replays_xai_models_corpus(#[files("fuzz/corpus/xai_models/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(models) = decode_xai_models(200, &bytes) {
        assert!(models.iter().all(|model| model.starts_with("xai/")));
    }
}
