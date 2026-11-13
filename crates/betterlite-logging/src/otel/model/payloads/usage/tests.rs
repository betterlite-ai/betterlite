#![cfg(test)]

use super::*;
#[test]
fn usage_parses_counts_and_computed_properties() {
    let value = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"prompt_tokens":10,"completion_tokens":6,"total_tokens":16,"metadata":{"usage_object":{"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}}}}"#).unwrap();
    let usage = LLMUsage::from_payload(&value);
    assert_eq!(usage.input_tokens, Some(10));
    assert_eq!(usage.output_tokens, Some(6));
    assert_eq!(usage.total_tokens, Some(16));
    assert_eq!(usage.uncached_input_tokens(), Some(8));
    assert_eq!(usage.non_reasoning_output_tokens(), Some(5));
}

#[test]
fn usage_skips_zero_cache_creation_before_fallback_source() {
    let value = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"prompt_tokens":20,"metadata":{"usage_object":{"cache_creation_input_tokens":0,"prompt_tokens_details":{"cache_creation_input_tokens":7}}}}"#).unwrap();
    let usage = LLMUsage::from_payload(&value);
    assert_eq!(usage.cache_creation_input_tokens, Some(7));
    assert_eq!(usage.uncached_input_tokens(), Some(13));
}

#[rstest::rstest]
#[case::absent("{}", None, None)]
#[case::clamp(r#"{"prompt_tokens":1,"completion_tokens":2,"metadata":{"usage_object":{"cache_read_input_tokens":5,"completion_tokens_details":{"reasoning_tokens":4}}}}"#, Some(0), Some(0))]
#[case::both_caches(r#"{"prompt_tokens":20,"completion_tokens":8,"metadata":{"usage_object":{"cache_creation_input_tokens":3,"cache_read_input_tokens":4}}}"#, Some(13), Some(8))]
fn computed_usage_handles_absence_and_clamping(
    #[case] raw: &str,
    #[case] input: Option<i64>,
    #[case] output: Option<i64>,
) {
    let usage = LLMUsage::from_payload(&serde_json::from_str(raw).unwrap());
    assert_eq!(usage.uncached_input_tokens(), input);
    assert_eq!(usage.non_reasoning_output_tokens(), output);
}

#[rstest::rstest]
#[case::creation_direct(
    r#"{"metadata":{"usage_object":{"cache_creation_input_tokens":3}}}"#,
    Some(3),
    None
)]
#[case::cache_write(
    r#"{"metadata":{"usage_object":{"prompt_tokens_details":{"cache_write_tokens":4}}}}"#,
    Some(4),
    None
)]
#[case::cache_creation(
    r#"{"metadata":{"usage_object":{"prompt_tokens_details":{"cache_creation_tokens":5}}}}"#,
    Some(5),
    None
)]
#[case::creation_details(
    r#"{"metadata":{"usage_object":{"prompt_tokens_details":{"cache_creation_input_tokens":6}}}}"#,
    Some(6),
    None
)]
#[case::read_direct(
    r#"{"metadata":{"usage_object":{"cache_read_input_tokens":7}}}"#,
    None,
    Some(7)
)]
#[case::cached_details(
    r#"{"metadata":{"usage_object":{"prompt_tokens_details":{"cached_tokens":8}}}}"#,
    None,
    Some(8)
)]
#[case::cache_hit(
    r#"{"metadata":{"usage_object":{"prompt_cache_hit_tokens":9}}}"#,
    None,
    Some(9)
)]
fn usage_reads_cache_sources(
    #[case] raw: &str,
    #[case] creation: Option<i64>,
    #[case] read: Option<i64>,
) {
    let usage = LLMUsage::from_payload(&serde_json::from_str(raw).unwrap());
    assert_eq!(
        (
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens
        ),
        (creation, read)
    );
}
