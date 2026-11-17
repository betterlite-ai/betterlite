#![cfg(test)]

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use rstest::rstest;

use super::{allowlist_enforced, server_applies_tool_allowlist, tool_result_error_message};

fn info(json: &str) -> MCPInfo {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> JsonValue {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::enforced(r#"{"tool_allowlist_enforced": true}"#, true)]
#[case::not_enforced(r#"{"tool_allowlist_enforced": false}"#, false)]
#[case::another_key(r#"{"protocol_version": "2025-03-26"}"#, false)]
#[case::empty_object("{}", false)]
#[case::string_flag(r#"{"tool_allowlist_enforced": "true"}"#, false)]
#[case::number_flag(r#"{"tool_allowlist_enforced": 1}"#, false)]
#[case::null_flag(r#"{"tool_allowlist_enforced": null}"#, false)]
fn allowlist_enforced_reads_only_the_json_true_switch(#[case] json: &str, #[case] expected: bool) {
    assert_eq!(allowlist_enforced(Some(&info(json))), expected);
}

#[test]
fn allowlist_enforced_is_false_without_mcp_info() {
    assert!(!allowlist_enforced(None));
}

#[test]
fn server_applies_tool_allowlist_turns_on_for_the_switch() {
    let switched = info(r#"{"tool_allowlist_enforced": true}"#);
    assert!(server_applies_tool_allowlist(Some(&switched), None));
    assert!(server_applies_tool_allowlist(Some(&switched), Some(&[])));
}

#[test]
fn server_applies_tool_allowlist_turns_on_for_a_non_empty_list() {
    let tools = ["get_pets".to_owned()];
    assert!(server_applies_tool_allowlist(None, Some(&tools)));
}

#[test]
fn server_applies_tool_allowlist_is_false_without_either_signal() {
    assert!(!server_applies_tool_allowlist(None, None));
    assert!(!server_applies_tool_allowlist(None, Some(&[])));
    let unswitched = info(r#"{"tool_allowlist_enforced": false}"#);
    assert!(!server_applies_tool_allowlist(Some(&unswitched), None));
}

#[test]
fn tool_result_error_message_returns_the_first_text_of_an_error() {
    let result = value(r#"{"isError": true, "content": [{"type": "text", "text": "boom"}]}"#);
    assert_eq!(tool_result_error_message(&result), Some("boom".to_owned()));
}

#[test]
fn tool_result_error_message_skips_empty_and_missing_texts() {
    let result = value(
        r#"{"isError": true, "content": [{"type": "text", "text": ""}, "note", {"text": 3}, {"text": "boom"}]}"#,
    );
    assert_eq!(tool_result_error_message(&result), Some("boom".to_owned()));
}

#[test]
fn tool_result_error_message_falls_back_without_a_usable_text() {
    let empty = value(r#"{"isError": true, "content": [{"text": ""}]}"#);
    assert_eq!(
        tool_result_error_message(&empty),
        Some("MCP tool call returned isError=true".to_owned())
    );
    let missing = value(r#"{"isError": true}"#);
    assert_eq!(
        tool_result_error_message(&missing),
        Some("MCP tool call returned isError=true".to_owned())
    );
    let not_an_array = value(r#"{"isError": true, "content": "boom"}"#);
    assert_eq!(
        tool_result_error_message(&not_an_array),
        Some("MCP tool call returned isError=true".to_owned())
    );
}

#[test]
fn tool_result_error_message_reads_is_error_when_is_error_camel_is_absent() {
    let result = value(r#"{"is_error": true, "content": [{"text": "boom"}]}"#);
    assert_eq!(tool_result_error_message(&result), Some("boom".to_owned()));
}

#[test]
fn tool_result_error_message_reads_is_error_when_is_error_camel_is_null() {
    let result = value(r#"{"isError": null, "is_error": true, "content": [{"text": "boom"}]}"#);
    assert_eq!(tool_result_error_message(&result), Some("boom".to_owned()));
}

#[test]
fn tool_result_error_message_keeps_a_false_is_error_camel_over_is_error() {
    let result = value(r#"{"isError": false, "is_error": true, "content": [{"text": "boom"}]}"#);
    assert_eq!(tool_result_error_message(&result), None);
}

#[rstest]
#[case::no_flag("{}")]
#[case::false_camel(r#"{"isError": false}"#)]
#[case::false_snake(r#"{"is_error": false}"#)]
#[case::null_camel(r#"{"isError": null}"#)]
#[case::text_flag(r#"{"isError": "true"}"#)]
#[case::number_flag(r#"{"isError": 1}"#)]
#[case::array("[]")]
#[case::text(r#""boom""#)]
#[case::null("null")]
fn tool_result_error_message_returns_none_for_a_non_error(#[case] json: &str) {
    assert_eq!(tool_result_error_message(&value(json)), None);
}
