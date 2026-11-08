#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::HiddenParamsError;
use super::get_additional_headers;
use super::get_hidden_params;
use crate::fields::payload::records::StandardLoggingHiddenParams;
use crate::logging_metadata::decode::DecodeError;

fn headers(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("headers decode")
}

fn hidden(encoded: &str) -> Json {
    serde_json::from_str(encoded).expect("hidden params decode")
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn absent_additional_headers_stay_absent() {
    assert_eq!(get_additional_headers(None), None);
}

#[test]
fn additional_headers_convert_the_typed_fields_from_hyphenated_names() {
    let converted = get_additional_headers(Some(&headers(
        r#"{"x-ratelimit-limit-requests": 5, "x-ratelimit-remaining-tokens": "7", "x-ratelimit-reset-requests": "6m0s"}"#,
    )))
    .expect("headers convert");
    assert_eq!(
        converted.get("x_ratelimit_limit_requests"),
        Some(&number(5))
    );
    assert_eq!(
        converted.get("x_ratelimit_remaining_tokens"),
        Some(&number(7))
    );
    assert_eq!(
        converted.get("x_ratelimit_reset_requests"),
        Some(&Json::Text("6m0s".to_owned()))
    );
}

#[rstest]
#[case::float_truncates(r#"{"x-ratelimit-limit-tokens": 5.9}"#, number(5))]
#[case::negative_float_truncates_toward_zero(r#"{"x-ratelimit-limit-tokens": -5.9}"#, number(-5))]
#[case::numeric_text(r#"{"x-ratelimit-limit-tokens": " 12 "}"#, number(12))]
#[case::signed_text(r#"{"x-ratelimit-limit-tokens": "+8"}"#, number(8))]
#[case::exponent_number(r#"{"x-ratelimit-limit-tokens": 1e3}"#, number(1_000))]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str, #[case] expected: Json) {
    let converted = get_additional_headers(Some(&headers(encoded))).expect("headers convert");
    assert_eq!(converted.get("x_ratelimit_limit_tokens"), Some(&expected));
}

#[rstest]
#[case::word("abc")]
#[case::decimal_text("1.5")]
#[case::empty("")]
fn unparsable_values_stay_as_sent(#[case] value: &str) {
    let encoded = format!(r#"{{"x-ratelimit-limit-tokens": "{value}"}}"#);
    let converted = get_additional_headers(Some(&headers(&encoded))).expect("headers convert");
    assert_eq!(
        converted.get("x_ratelimit_limit_tokens"),
        Some(&Json::Text(value.to_owned()))
    );
}

#[test]
fn untyped_headers_are_preserved_verbatim() {
    let converted = get_additional_headers(Some(&headers(
        r#"{"llm_provider-x-request-id": "req-1", "X-RateLimit-Limit-Requests": 9, "x_ratelimit_limit_requests": 3}"#,
    )))
    .expect("headers convert");
    assert_eq!(
        converted.get("llm_provider-x-request-id"),
        Some(&Json::Text("req-1".to_owned()))
    );
    assert_eq!(
        converted.get("X-RateLimit-Limit-Requests"),
        Some(&number(9))
    );
    assert_eq!(
        converted.get("x_ratelimit_limit_requests"),
        Some(&number(3))
    );
}

#[test]
fn absent_hidden_params_produce_an_empty_record() {
    let record = get_hidden_params(None).expect("record");
    assert_eq!(record, StandardLoggingHiddenParams::default());
}

#[test]
fn hidden_params_copy_declared_fields_and_drop_unknown_ones() {
    let record = get_hidden_params(Some(&hidden(
        r#"{"model_id": "m", "cache_key": "c", "unknown": "x", "batch_models": ["a"]}"#,
    )))
    .expect("record");
    assert_eq!(record.model_id.as_deref(), Some("m"));
    assert_eq!(record.cache_key.as_deref(), Some("c"));
    assert_eq!(record.batch_models, Some(vec!["a".to_owned()]));
    let encoded = serde_json::to_string(&record).expect("record encodes");
    assert!(!encoded.contains("unknown"));
}

#[test]
fn hidden_params_convert_additional_headers() {
    let record = get_hidden_params(Some(&hidden(
        r#"{"additional_headers": {"x-ratelimit-limit-requests": "4", "llm_provider-x-request-id": "r"}}"#,
    )))
    .expect("record");
    let converted = record.additional_headers.expect("headers");
    assert_eq!(
        converted.get("x_ratelimit_limit_requests"),
        Some(&number(4))
    );
    assert_eq!(
        converted.get("llm_provider-x-request-id"),
        Some(&Json::Text("r".to_owned()))
    );
}

#[test]
fn null_additional_headers_stay_absent() {
    let record =
        get_hidden_params(Some(&hidden(r#"{"additional_headers": null}"#))).expect("record");
    assert_eq!(record.additional_headers, None);
}

#[test]
fn a_mistyped_additional_headers_value_is_refused() {
    let error =
        get_hidden_params(Some(&hidden(r#"{"additional_headers": 7}"#))).expect_err("wrong type");
    assert!(matches!(error, HiddenParamsError::AdditionalHeadersType));
}

#[test]
fn a_mistyped_hidden_param_is_reported_by_name() {
    let error = get_hidden_params(Some(&hidden(r#"{"model_id": 7}"#))).expect_err("wrong type");
    let HiddenParamsError::Decode(DecodeError::Field { path, .. }) = error else {
        panic!("a field error is expected");
    };
    assert_eq!(path, "model_id");
}

#[test]
fn non_object_hidden_params_are_absent() {
    let record = get_hidden_params(Some(&hidden("[]"))).expect("record");
    assert_eq!(record, StandardLoggingHiddenParams::default());
}
