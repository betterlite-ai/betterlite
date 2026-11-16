#![cfg(test)]

use crate::mcp_stored_contracts::JsonValue;
use crate::search_tools::tests::fakes::stored_record;

#[test]
fn record_debug_output_never_prints_the_stored_parameters() {
    let mut record = stored_record("tool-1", "web");
    record.litellm_params = JsonValue::String(r#"{"api_key":"sk-secret"}"#.to_owned());
    let text = format!("{record:?}");
    assert!(text.contains("tool-1"));
    assert!(!text.contains("sk-secret"));
}

#[test]
fn record_clones_compare_equal_and_differ_by_field() {
    let record = stored_record("tool-1", "web");
    assert_eq!(record.clone(), record);
    assert_ne!(record, stored_record("tool-2", "web"));
}
