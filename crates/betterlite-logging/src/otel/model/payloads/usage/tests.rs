#![cfg(test)]

use super::*;
#[test]
fn usage_parses_counts_and_computed_properties() {
    let value = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"prompt_tokens":10,"completion_tokens":6,"total_tokens":16,"metadata":{"usage_object":{"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}}}}"#).unwrap();
    let usage = LLMUsage::from_payload(&value);
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
