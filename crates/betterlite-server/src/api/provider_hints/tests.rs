#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_body::JsonObject;
use http::header::HeaderName;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::{provider_from_body, provider_from_headers, provider_from_query};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("a test header name"),
            HeaderValue::from_bytes(value.as_bytes()).expect("a test header value"),
        );
    }
    map
}

fn body_with(provider: JsonValue) -> JsonObject {
    JsonObject::from([(String::from("custom_llm_provider"), provider)])
}

#[test]
fn header_absent_is_none() {
    assert_eq!(provider_from_headers(&HeaderMap::new()), None);
}

#[test]
fn header_empty_value_is_some_empty() {
    let map = headers(&[("custom-llm-provider", "")]);
    assert_eq!(provider_from_headers(&map), Some(String::new()));
}

#[test]
fn header_repeats_keep_the_first() {
    let map = headers(&[
        ("custom-llm-provider", "first"),
        ("custom-llm-provider", "second"),
    ]);
    assert_eq!(provider_from_headers(&map), Some(String::from("first")));
}

#[test]
fn header_name_matches_case_insensitively() {
    let map = headers(&[("CUSTOM-LLM-PROVIDER", "bedrock")]);
    assert_eq!(provider_from_headers(&map), Some(String::from("bedrock")));
}

#[test]
fn header_with_underscores_does_not_match() {
    let map = headers(&[("custom_llm_provider", "bedrock")]);
    assert_eq!(provider_from_headers(&map), None);
}

#[test]
fn header_byte_above_ascii_decodes_as_latin1() {
    let mut map = HeaderMap::new();
    map.insert(
        "custom-llm-provider",
        HeaderValue::from_bytes(b"bedrock\xFF").expect("a latin-1 header value"),
    );
    assert_eq!(
        provider_from_headers(&map),
        Some(String::from("bedrock\u{ff}"))
    );
}

#[test]
fn query_absent_is_none() {
    assert_eq!(provider_from_query(None), None);
}

#[test]
fn query_empty_string_is_none() {
    assert_eq!(provider_from_query(Some("")), None);
}

#[test]
fn query_without_the_key_is_none() {
    assert_eq!(provider_from_query(Some("model=gpt-4")), None);
}

#[test]
fn query_blank_value_is_some_empty() {
    assert_eq!(
        provider_from_query(Some("custom_llm_provider=")),
        Some(String::new())
    );
}

#[test]
fn query_bare_key_is_some_empty() {
    assert_eq!(
        provider_from_query(Some("custom_llm_provider")),
        Some(String::new())
    );
}

#[test]
fn query_repeats_keep_the_last() {
    assert_eq!(
        provider_from_query(Some("custom_llm_provider=first&custom_llm_provider=second")),
        Some(String::from("second"))
    );
}

#[test]
fn query_decodes_plus_and_percent_escapes() {
    assert_eq!(
        provider_from_query(Some("custom_llm_provider=azure%2Fopen+ai")),
        Some(String::from("azure/open ai"))
    );
}

#[test]
fn query_hyphen_spelling_does_not_match() {
    assert_eq!(
        provider_from_query(Some("custom-llm-provider=bedrock")),
        None
    );
}

#[rstest]
#[case::empty_string(JsonValue::String(String::new()))]
#[case::null(JsonValue::Null)]
#[case::number(JsonValue::from(123_i32))]
#[case::provider_name(JsonValue::String(String::from("bedrock")))]
fn body_field_value_comes_back_unchanged(#[case] provider: JsonValue) {
    let body = body_with(provider.clone());
    assert_eq!(provider_from_body(&body), Some(&provider));
}

#[test]
fn body_absent_is_none() {
    assert_eq!(provider_from_body(&JsonObject::new()), None);
}

#[test]
fn body_other_fields_are_ignored() {
    let body = JsonObject::from([(
        String::from("model"),
        JsonValue::String(String::from("gpt-4")),
    )]);
    assert_eq!(provider_from_body(&body), None);
}

#[test]
fn body_hyphen_spelling_does_not_match() {
    let body = JsonObject::from([(
        String::from("custom-llm-provider"),
        JsonValue::String(String::from("bedrock")),
    )]);
    assert_eq!(provider_from_body(&body), None);
}
