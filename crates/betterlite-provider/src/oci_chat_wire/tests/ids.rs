use rstest::rstest;

use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};

use super::support::expected_call_id;

#[rstest]
#[case::none(None, None)]
#[case::complete(Some("COMPLETE"), Some("stop"))]
#[case::max_tokens(Some("MAX_TOKENS"), Some("length"))]
#[case::tool_call(Some("TOOL_CALL"), Some("tool_calls"))]
#[case::tool_calls(Some("TOOL_CALLS"), Some("tool_calls"))]
#[case::cancelled(Some("CANCELLED"), Some("stop"))]
#[case::empty(Some(""), Some("stop"))]
fn finish_reasons_map_to_openai_values(#[case] raw: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(normalize_oci_finish_reason(raw).as_deref(), expected);
}

#[test]
fn synthesized_tool_call_ids_hash_position_name_and_arguments() {
    let id = synthesize_oci_tool_call_id(3, "lookup", "{\"a\":1}");
    assert_eq!(id, expected_call_id(3, "lookup", "{\"a\":1}"));
    assert_eq!(id.len(), 29);
    assert_ne!(id, synthesize_oci_tool_call_id(4, "lookup", "{\"a\":1}"));
    assert_ne!(id, synthesize_oci_tool_call_id(3, "other", "{\"a\":1}"));
}
