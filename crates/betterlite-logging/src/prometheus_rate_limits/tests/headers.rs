#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{json_object, number, payload, with_headers};
use crate::prometheus_rate_limits::headers::{
    declared_integer, get_int_from_v3_rate_limit_headers, get_remaining_from_v3_rate_limit_headers,
};
use crate::prometheus_rate_limits::metric::RateLimitType;

#[test]
fn v3_remaining_reads_the_model_per_key_headers() {
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-model_per_key-remaining-requests", number("7")),
            ("x-ratelimit-model_per_key-remaining-tokens", number("9")),
        ],
    );
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(Some(&payload), RateLimitType::Requests),
        Some(7)
    );
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(Some(&payload), RateLimitType::Tokens),
        Some(9)
    );
}

#[test]
fn v3_remaining_is_none_without_a_payload() {
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(None, RateLimitType::Requests),
        None
    );
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(None, RateLimitType::Tokens),
        None
    );
}

#[test]
fn v3_remaining_is_none_for_a_missing_header() {
    let payload = with_headers(payload(), &[]);
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(Some(&payload), RateLimitType::Requests),
        None
    );
}

#[test]
fn v3_remaining_ignores_the_key_team_and_response_header_families() {
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-api_key-remaining-requests", number("3")),
            ("x-ratelimit-team-remaining-requests", number("4")),
            ("x_ratelimit_remaining_requests", number("5")),
            ("x_ratelimit_remaining_tokens", number("6")),
        ],
    );
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(Some(&payload), RateLimitType::Requests),
        None
    );
    assert_eq!(
        get_remaining_from_v3_rate_limit_headers(Some(&payload), RateLimitType::Tokens),
        None
    );
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-3", -3)]
#[case::positive("42", 42)]
fn v3_int_reads_each_integer(#[case] literal: &str, #[case] expected: i64) {
    let payload = with_headers(payload(), &[("x-test-header", number(literal))]);
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        Some(expected)
    );
}

#[rstest]
#[case::true_value(true)]
#[case::false_value(false)]
fn v3_int_rejects_booleans(#[case] value: bool) {
    let payload = with_headers(payload(), &[("x-test-header", Json::Bool(value))]);
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        None
    );
}

#[rstest]
#[case::text(Json::Text("5".to_owned()))]
#[case::float(number("1.5"))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(json_object(&[])))]
fn v3_int_rejects_non_integers(#[case] value: Json) {
    let payload = with_headers(payload(), &[("x-test-header", value)]);
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        None
    );
}

#[test]
fn v3_int_reads_a_null_header_as_missing() {
    let payload = with_headers(payload(), &[("x-test-header", Json::Null)]);
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        None
    );
}

#[test]
fn v3_int_reads_an_integer_out_of_i64_range_as_missing() {
    let payload = with_headers(
        payload(),
        &[("x-test-header", number("9223372036854775808"))],
    );
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        None
    );
}

#[test]
fn v3_int_is_none_without_additional_headers() {
    let payload = payload();
    assert!(payload.hidden_params.additional_headers.is_none());
    assert_eq!(
        get_int_from_v3_rate_limit_headers(&payload, "x-test-header"),
        None
    );
}

#[test]
fn declared_integer_reads_absent_and_null_as_none() {
    assert_eq!(declared_integer(None, "field").unwrap(), None);
    assert_eq!(declared_integer(Some(&Json::Null), "field").unwrap(), None);
}

#[rstest]
#[case::boolean(Json::Bool(true))]
#[case::text(Json::Text("5".to_owned()))]
#[case::float(number("1.5"))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(json_object(&[])))]
#[case::out_of_range(number("9223372036854775808"))]
fn declared_integer_rejects_a_value_it_cannot_hold(#[case] value: Json) {
    let error = declared_integer(Some(&value), "metadata.key").unwrap_err();
    assert_eq!(
        error.to_string(),
        "the metadata.key value must be an integer"
    );
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-4", -4)]
fn declared_integer_reads_an_integer(#[case] literal: &str, #[case] expected: i64) {
    assert_eq!(
        declared_integer(Some(&number(literal)), "field").unwrap(),
        Some(expected)
    );
}
