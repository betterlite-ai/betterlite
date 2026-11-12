#![cfg(test)]

use super::search_requests_from_response;
use crate::decisions::Json;

fn from_json(text: &str) -> Json {
    serde_json::from_str(text).expect("a fixture response parses")
}

fn response(web_search_requests: &str) -> Json {
    from_json(&format!(
        r#"{{"usage":{{"server_tool_use":{{"web_search_requests":{web_search_requests}}}}}}}"#
    ))
}

#[rstest::rstest]
#[case::integer("3", Some(3))]
#[case::integer_string(r#""3""#, Some(3))]
#[case::padded_string(r#"" 3 ""#, Some(3))]
#[case::signed_string(r#""+3""#, Some(3))]
#[case::negative_string(r#""-3""#, Some(-3))]
#[case::integral_float_string(r#""3.0""#, Some(3))]
#[case::zero_fraction_string(r#""03.00""#, Some(3))]
#[case::underscored_string(r#""1_0""#, Some(10))]
#[case::bool_true("true", Some(1))]
#[case::bool_false("false", Some(0))]
#[case::float_integral("3.0", Some(3))]
#[case::float_fraction("3.5", None)]
#[case::null("null", None)]
#[case::fraction_string(r#""3.5""#, None)]
#[case::double_underscore(r#""1__0""#, None)]
#[case::leading_underscore(r#""_1""#, None)]
#[case::trailing_underscore(r#""1_""#, None)]
#[case::fraction_underscore(r#""3.0_0""#, None)]
#[case::exponent_string(r#""1e3""#, None)]
#[case::empty_string(r#""""#, None)]
#[case::array("[]", None)]
#[case::object("{}", None)]
fn coercion_matches_the_pinned_pydantic_field(#[case] input: &str, #[case] expected: Option<i64>) {
    assert_eq!(search_requests_from_response(&response(input)), expected);
}

#[test]
fn a_present_zero_is_reported() {
    assert_eq!(
        search_requests_from_response(&from_json(
            r#"{"usage":{"server_tool_use":{"web_search_requests":0}}}"#
        )),
        Some(0)
    );
}

#[test]
fn a_missing_usage_block_is_absent() {
    assert_eq!(search_requests_from_response(&from_json("{}")), None);
}

#[test]
fn a_null_usage_block_is_absent() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":null}"#)),
        None
    );
}

#[test]
fn a_missing_server_tool_use_is_absent() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":{}}"#)),
        None
    );
}

#[test]
fn a_null_server_tool_use_is_absent() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":{"server_tool_use":null}}"#)),
        None
    );
}

#[test]
fn a_missing_counter_is_absent() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":{"server_tool_use":{}}}"#)),
        None
    );
}

#[test]
fn a_non_mapping_usage_block_fails_validation() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":"x"}"#)),
        None
    );
}

#[test]
fn a_non_mapping_server_tool_use_fails_validation() {
    assert_eq!(
        search_requests_from_response(&from_json(r#"{"usage":{"server_tool_use":3}}"#)),
        None
    );
}

#[rstest::rstest]
#[case::list("[]")]
#[case::text(r#""response""#)]
#[case::integer("3")]
#[case::null("null")]
fn only_a_mapping_response_is_eligible(#[case] text: &str) {
    assert_eq!(search_requests_from_response(&from_json(text)), None);
}

#[test]
fn unknown_fields_are_ignored() {
    assert_eq!(
        search_requests_from_response(&from_json(
            r#"{"id":"msg","usage":{"input_tokens":1,"server_tool_use":{"web_search_requests":2,"tool_search_requests":"bad"}}}"#
        )),
        Some(2)
    );
}

#[test]
fn an_invalid_nested_count_rejects_the_whole_probe() {
    assert_eq!(
        search_requests_from_response(&from_json(
            r#"{"usage":{"server_tool_use":{"web_search_requests":"abc"}}}"#
        )),
        None
    );
}

#[test]
fn an_integer_beyond_the_signed_range_is_absent() {
    assert_eq!(
        search_requests_from_response(&from_json(
            r#"{"usage":{"server_tool_use":{"web_search_requests":9223372036854775808}}}"#
        )),
        None
    );
}
