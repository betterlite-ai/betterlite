#![cfg(test)]

use rstest::rstest;

use super::{UsageObject, extract_cache_creation_tokens, extract_cache_read_tokens};

fn usage(text: &str) -> UsageObject {
    serde_json::from_str(text).expect("the fixture usage object deserializes")
}

enum CacheField {
    Read,
    Creation,
}

fn extract(field: &CacheField, value: &UsageObject) -> Result<i64, super::SavingsEvidenceError> {
    match field {
        CacheField::Read => extract_cache_read_tokens(Some(value)),
        CacheField::Creation => extract_cache_creation_tokens(Some(value)),
    }
}

#[rstest]
#[case::read_top_level(CacheField::Read, r#"{"cache_read_input_tokens": 12}"#, 12)]
#[case::read_top_level_negative(CacheField::Read, r#"{"cache_read_input_tokens": -3}"#, -3)]
#[case::read_top_level_zero_falls_through(
    CacheField::Read,
    r#"{"cache_read_input_tokens": 0, "prompt_tokens_details": {"cached_tokens": 9}}"#,
    9
)]
#[case::read_fractional_is_truthy_and_truncates(
    CacheField::Read,
    r#"{"cache_read_input_tokens": 0.4, "prompt_tokens_details": {"cached_tokens": 9}}"#,
    0
)]
#[case::read_detail_fallback(
    CacheField::Read,
    r#"{"prompt_tokens_details": {"cached_tokens": 5}}"#,
    5
)]
#[case::read_missing_details(CacheField::Read, "{}", 0)]
#[case::creation_top_level(CacheField::Creation, r#"{"cache_creation_input_tokens": 7}"#, 7)]
#[case::creation_write_alias_wins_over_creation(
    CacheField::Creation,
    r#"{"prompt_tokens_details": {"cache_write_tokens": 4, "cache_creation_tokens": 9}}"#,
    4
)]
#[case::creation_falls_back_to_creation_when_write_is_falsy(
    CacheField::Creation,
    r#"{"prompt_tokens_details": {"cache_write_tokens": 0, "cache_creation_tokens": 9}}"#,
    9
)]
#[case::creation_missing(CacheField::Creation, "{}", 0)]
fn extracts_cache_tokens_with_top_level_then_detail_precedence(
    #[case] field: CacheField,
    #[case] text: &str,
    #[case] expected: i64,
) {
    let value = usage(text);
    assert_eq!(
        extract(&field, &value).expect("fixture does not overflow"),
        expected
    );
}

#[test]
fn extract_cache_read_tokens_is_zero_without_a_usage_object() {
    assert_eq!(extract_cache_read_tokens(None).expect("no overflow"), 0);
}

#[test]
fn extract_cache_read_tokens_accepts_a_boolean_top_level_marker() {
    let value = usage(r#"{"cache_read_input_tokens": true}"#);
    assert_eq!(
        extract_cache_read_tokens(Some(&value)).expect("no overflow"),
        1
    );
}
