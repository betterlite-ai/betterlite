#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{parse_results, safe_int, safe_response_text};
use crate::compresr_transport::failure::{BATCH_MISMATCH, UNEXPECTED_SHAPE, UNREADABLE};

fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn body(source: &str) -> Option<&str> {
    Some(source)
}

#[rstest]
#[case::signed(r#"42"#, 42)]
#[case::negative(r#"-5"#, -5)]
#[case::unsigned_large(r#"18446744073709551615"#, i64::MAX)]
#[case::decimal(r#"4.9"#, 4)]
#[case::negative_decimal(r#"-4.9"#, -4)]
#[case::numeric_text(r#""42""#, 42)]
#[case::padded_text(r#"" 42 ""#, 42)]
#[case::non_numeric_text(r#""lots""#, 0)]
#[case::float_text(r#""4.5""#, 0)]
#[case::null("null", 0)]
#[case::boolean("true", 0)]
#[case::list("[]", 0)]
#[case::object("{}", 0)]
fn token_stats_parse_defensively(#[case] source: &str, #[case] expected: i64) {
    assert_eq!(safe_int(Some(&value(source))), expected);
}

#[test]
fn a_missing_token_stat_is_zero() {
    assert_eq!(safe_int(None), 0);
}

#[test]
fn an_undecodable_body_becomes_a_marker() {
    assert_eq!(safe_response_text(None, 500), "<undecodable response body>");
}

#[rstest]
#[case::short("hello", 500, "hello")]
#[case::exact("hello", 5, "hello")]
#[case::truncated("hello", 4, "hell")]
#[case::empty("", 500, "")]
#[case::characters_not_bytes("ééé", 2, "éé")]
fn response_text_is_bounded_to_the_limit(
    #[case] text: &str,
    #[case] limit: usize,
    #[case] expected: &str,
) {
    assert_eq!(safe_response_text(Some(text), limit), expected);
}

#[test]
fn a_single_response_is_its_own_result() {
    let results = parse_results(
        body(
            r#"{"data":{"compressed_context":"short","original_tokens":10,"compressed_tokens":4}}"#,
        ),
        1,
    )
    .expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].compressed_context.as_deref(), Some("short"));
    assert_eq!(results[0].original_tokens, 10);
    assert_eq!(results[0].compressed_tokens, 4);
}

#[test]
fn a_batch_response_keeps_one_result_per_input_in_order() {
    let results = parse_results(
        body(
            r#"{"data":{"results":[
                {"compressed_context":"one","original_tokens":5,"compressed_tokens":1},
                {"compressed_context":"two","original_tokens":6,"compressed_tokens":2}
            ]}}"#,
        ),
        2,
    )
    .expect("results");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].compressed_context.as_deref(), Some("one"));
    assert_eq!(results[1].compressed_context.as_deref(), Some("two"));
    assert_eq!(results[1].compressed_tokens, 2);
}

#[rstest]
#[case::too_few(r#"{"data":{"results":[{"compressed_context":"one"}]}}"#, 2)]
#[case::too_many(
    r#"{"data":{"results":[{"compressed_context":"one"},{"compressed_context":"two"},{"compressed_context":"three"}]}}"#,
    2
)]
#[case::missing_results(r#"{"data":{}}"#, 2)]
#[case::null_results(r#"{"data":{"results":null}}"#, 2)]
#[case::not_a_list(r#"{"data":{"results":"one"}}"#, 2)]
#[case::non_object_result(r#"{"data":{"results":["one","two"]}}"#, 2)]
fn a_batch_without_one_object_per_input_fails(#[case] source: &str, #[case] expected: usize) {
    let failure = parse_results(body(source), expected).expect_err("failure");
    assert_eq!(failure.message, BATCH_MISMATCH);
}

#[rstest]
#[case::list("[]")]
#[case::scalar("5")]
#[case::no_data(r#"{"results":[]}"#)]
#[case::data_not_an_object(r#"{"data":[]}"#)]
fn an_unexpected_body_shape_fails(#[case] source: &str) {
    let failure = parse_results(body(source), 1).expect_err("failure");
    assert_eq!(failure.message, UNEXPECTED_SHAPE);
}

#[rstest]
#[case::not_json("{")]
fn an_unreadable_body_fails(#[case] source: &str) {
    let failure = parse_results(body(source), 1).expect_err("failure");
    assert_eq!(failure.message, UNREADABLE);
}

#[test]
fn an_unread_body_is_unreadable() {
    let failure = parse_results(None, 1).expect_err("failure");
    assert_eq!(failure.message, UNREADABLE);
}
