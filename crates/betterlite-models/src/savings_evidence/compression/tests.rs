#![cfg(test)]

use rstest::rstest;

use super::compression_saved_tokens;
use crate::savings_evidence::metadata::Metadata;

fn metadata(text: &str) -> Metadata {
    serde_json::from_str(text).expect("the fixture metadata deserializes")
}

#[rstest]
#[case::native_only(r#"{"compression_savings": {"tokens_saved": 40}}"#, 40)]
#[case::native_float_truncates(r#"{"compression_savings": {"tokens_saved": 40.9}}"#, 40)]
#[case::native_bool_is_zero(r#"{"compression_savings": {"tokens_saved": true}}"#, 0)]
#[case::native_negative_is_zero(r#"{"compression_savings": {"tokens_saved": -5}}"#, 0)]
#[case::native_numeric_string_is_zero(r#"{"compression_savings": {"tokens_saved": "40"}}"#, 0)]
#[case::missing_metadata("{}", 0)]
fn compression_saved_tokens_reads_native_savings(#[case] text: &str, #[case] expected: i64) {
    assert_eq!(
        compression_saved_tokens(&metadata(text)).expect("fixture does not overflow"),
        expected
    );
}

#[test]
fn compression_saved_tokens_adds_a_single_headroom_mapping() {
    let value = metadata(
        r#"{
            "guardrail_information": {
                "guardrail_provider": "headroom",
                "guardrail_response": {"tokens_saved": 10}
            }
        }"#,
    );
    assert_eq!(compression_saved_tokens(&value).expect("no overflow"), 10);
}

#[test]
fn compression_saved_tokens_sums_a_list_of_headroom_entries() {
    let value = metadata(
        r#"{
            "guardrail_information": [
                {"guardrail_provider": "headroom", "guardrail_response": {"tokens_saved": 10}},
                {"guardrail_provider": "headroom", "guardrail_response": {"tokens_saved": 5}},
                {"guardrail_provider": "other", "guardrail_response": {"tokens_saved": 100}}
            ]
        }"#,
    );
    assert_eq!(compression_saved_tokens(&value).expect("no overflow"), 15);
}

#[test]
fn compression_saved_tokens_ignores_a_non_list_non_mapping_guardrail_shape() {
    let value = metadata(r#"{"guardrail_information": "not-a-collection"}"#);
    assert_eq!(compression_saved_tokens(&value).expect("no overflow"), 0);
}

#[test]
fn compression_saved_tokens_combines_native_and_headroom_totals() {
    let value = metadata(
        r#"{
            "compression_savings": {"tokens_saved": 7},
            "guardrail_information": {
                "guardrail_provider": "headroom",
                "guardrail_response": {"tokens_saved": 3}
            }
        }"#,
    );
    assert_eq!(compression_saved_tokens(&value).expect("no overflow"), 10);
}

#[test]
fn compression_saved_tokens_errors_on_an_unrepresentable_total() {
    // Parsed from JSON text, so arbitrary_precision keeps the oversized lexeme instead of rounding it.
    let value = metadata(r#"{"compression_savings":{"tokens_saved":1e400}}"#);
    compression_saved_tokens(&value).unwrap_err();
}

#[test]
fn compression_saved_tokens_errors_when_the_sum_overflows_i64() {
    let text = format!(
        r#"{{"guardrail_information": [
            {{"guardrail_provider": "headroom", "guardrail_response": {{"tokens_saved": {}}}}},
            {{"guardrail_provider": "headroom", "guardrail_response": {{"tokens_saved": 1}}}}
        ]}}"#,
        i64::MAX
    );
    compression_saved_tokens(&metadata(&text)).unwrap_err();
}
