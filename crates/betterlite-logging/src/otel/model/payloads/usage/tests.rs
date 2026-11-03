#![cfg(test)]

use super::*;
#[test]
fn usage_parses_counts_and_computed_properties() {
    let value = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"prompt_tokens":10,"completion_tokens":6,"total_tokens":16,"metadata":{"usage_object":{"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":1}}}}"#).unwrap();
    let usage = LLMUsage::from_payload(&value);
    assert_eq!(usage.uncached_input_tokens(), Some(8));
    assert_eq!(usage.non_reasoning_output_tokens(), Some(5));
}
