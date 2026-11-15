#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{parse_body, parse_results, safe_int, safe_response_text};
use crate::compresr_transport::failure::{BATCH_MISMATCH, UNEXPECTED_SHAPE, UNREADABLE};

#[rstest]
#[case::signed(GuardrailValue::SignedInteger(42), 42)]
#[case::negative(GuardrailValue::SignedInteger(-5), -5)]
#[case::unsigned_large(GuardrailValue::UnsignedInteger(u64::MAX), i64::MAX)]
#[case::decimal(GuardrailValue::Decimal(4.9_f64), 4)]
#[case::negative_decimal(GuardrailValue::Decimal(-4.9_f64), -4)]
#[case::numeric_text(GuardrailValue::Text("42".to_owned()), 42)]
#[case::padded_text(GuardrailValue::Text(" 42 ".to_owned()), 42)]
#[case::non_numeric_text(GuardrailValue::Text("lots".to_owned()), 0)]
#[case::float_text(GuardrailValue::Text("4.5".to_owned()), 0)]
#[case::underscored_text(GuardrailValue::Text("1_0".to_owned()), 0)]
#[case::null(GuardrailValue::Null, 0)]
#[case::boolean_true(GuardrailValue::Boolean(true), 1)]
#[case::boolean_false(GuardrailValue::Boolean(false), 0)]
#[case::list(GuardrailValue::Array(Vec::new()), 0)]
#[case::object(GuardrailValue::Object(BTreeMap::new()), 0)]
#[case::huge_decimal(GuardrailValue::Decimal(1e300_f64), 0)]
fn token_stats_parse_defensively(#[case] value: GuardrailValue, #[case] expected: i64) {
    assert_eq!(safe_int(Some(&value)), expected);
}

#[test]
fn a_missing_token_stat_is_zero() {
    assert_eq!(safe_int(None), 0);
}

#[test]
fn a_non_finite_token_stat_is_zero() {
    let nan = GuardrailValue::Decimal(f64::NAN);
    let infinity = GuardrailValue::Decimal(f64::INFINITY);
    assert_eq!(safe_int(Some(&nan)), 0);
    assert_eq!(safe_int(Some(&infinity)), 0);
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
#[case::characters_not_bytes("\u{e9}\u{e9}\u{e9}", 2, "\u{e9}\u{e9}")]
fn response_text_is_bounded_to_the_limit(
    #[case] text: &str,
    #[case] limit: usize,
    #[case] expected: &str,
) {
    assert_eq!(safe_response_text(Some(text), limit), expected);
}

#[test]
fn a_single_response_is_its_own_result() {
    let Ok(results) = parse_results(
        Some(
            r#"{"data":{"compressed_context":"short","original_tokens":10,"compressed_tokens":4}}"#,
        ),
        1,
    ) else {
        panic!("a single response must parse");
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].compressed_context.as_deref(), Some("short"));
    assert_eq!(results[0].original_tokens, 10);
    assert_eq!(results[0].compressed_tokens, 4);
}

#[test]
fn a_batch_response_keeps_one_result_per_input_in_order() {
    let Ok(results) = parse_results(
        Some(
            r#"{"data":{"results":[
                {"compressed_context":"one","original_tokens":5,"compressed_tokens":1},
                {"compressed_context":"two","original_tokens":6,"compressed_tokens":2}
            ]}}"#,
        ),
        2,
    ) else {
        panic!("a batch response must parse");
    };
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
    let failure = parse_results(Some(source), expected).expect_err("failure");
    assert_eq!(failure.message, BATCH_MISMATCH);
}

#[rstest]
#[case::list("[]")]
#[case::scalar("5")]
#[case::no_data(r#"{"results":[]}"#)]
#[case::data_not_an_object(r#"{"data":[]}"#)]
#[case::number_marker_key(r#"{"data":{"$serde_json::private::Number":{"x":1}}}"#)]
fn an_unexpected_body_shape_fails(#[case] source: &str) {
    let failure = parse_results(Some(source), 1).expect_err("failure");
    assert_eq!(failure.message, UNEXPECTED_SHAPE);
}

#[test]
fn an_unreadable_body_fails() {
    let failure = parse_results(Some("{"), 1).expect_err("failure");
    assert_eq!(failure.message, UNREADABLE);
}

#[test]
fn a_float_token_stat_in_the_body_is_read_as_a_number() {
    let Ok(results) = parse_results(
        Some(
            r#"{"data":{"compressed_context":"short","original_tokens":4.9,"compressed_tokens":-4.9}}"#,
        ),
        1,
    ) else {
        panic!("a response with float stats must parse");
    };
    assert_eq!(results[0].original_tokens, 4);
    assert_eq!(results[0].compressed_tokens, -4);
}

#[test]
fn a_token_stat_past_the_float_range_is_zero() {
    let Ok(results) = parse_results(
        Some(r#"{"data":{"compressed_context":"short","original_tokens":1e400}}"#),
        1,
    ) else {
        panic!("a response with an unrepresentable stat must still parse");
    };
    assert_eq!(results[0].compressed_context.as_deref(), Some("short"));
    assert_eq!(results[0].original_tokens, 0);
}

#[test]
fn an_unread_body_is_unreadable() {
    let failure = parse_results(None, 1).expect_err("failure");
    assert_eq!(failure.message, UNREADABLE);
}

#[test]
fn a_batch_result_reads_float_stats_as_numbers() {
    let Ok(results) = parse_results(
        Some(
            r#"{"data":{"results":[{"compressed_context":"one","original_tokens":2.5},{"compressed_context":"two","compressed_tokens":3.5}]}}"#,
        ),
        2,
    ) else {
        panic!("a batch response with float stats must parse");
    };
    assert_eq!(results[0].original_tokens, 2);
    assert_eq!(results[1].compressed_tokens, 3);
}

#[test]
fn replays_the_compresr_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/compresr_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        [1, 2].into_iter().any(|expected| {
            parse_body(&bytes, expected).is_some_and(|results| {
                !results.is_empty()
                    && results
                        .iter()
                        .all(|result| result.compressed_context.is_some())
            })
        })
    }));
}
