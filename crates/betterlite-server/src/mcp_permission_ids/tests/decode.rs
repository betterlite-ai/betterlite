use std::fs;
use std::path::PathBuf;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};
use rstest::rstest;

use crate::mcp_permission_ids::decode::{McpToolGrantsJsonError, decode_mcp_tool_grants_json};

use super::fixtures::{json, nested_array_grant};

// A grant document of exactly the requested byte length, with one long tool name.
fn grant_text_of_len(total: usize) -> String {
    let prefix = r#"{"srv-a": ""#;
    let suffix = "\"}";
    let filler = "x".repeat(
        total
            .saturating_sub(prefix.len())
            .saturating_sub(suffix.len()),
    );
    format!("{prefix}{filler}{suffix}")
}

#[test]
fn decodes_a_stored_grant_object() {
    let grants =
        decode_mcp_tool_grants_json(br#"{"srv-a": ["t1", "t2"], "srv-b": [], "srv-c": null}"#)
            .expect("the stored grant object decodes");
    assert_eq!(grants.get("srv-a"), Some(&json(r#"["t1", "t2"]"#)));
    assert_eq!(grants.get("srv-b"), Some(&json("[]")));
    assert_eq!(grants.get("srv-c"), Some(&JsonData::Null));
    assert!(
        decode_mcp_tool_grants_json(b"{}")
            .expect("an empty grant object decodes")
            .is_empty()
    );
}

#[rstest]
#[case::array(b"[]")]
#[case::string(br#""text""#)]
#[case::null(b"null")]
#[case::number(b"7")]
#[case::bool(b"true")]
#[case::zero(b"0")]
#[case::false_bool(b"false")]
fn non_object_documents_are_refused(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_mcp_tool_grants_json(bytes),
        Err(McpToolGrantsJsonError::NotAnObject)
    ));
}

#[test]
fn malformed_documents_are_refused() {
    let truncated =
        decode_mcp_tool_grants_json(br#"{"srv-a": "#).expect_err("truncated JSON is refused");
    assert!(matches!(
        truncated,
        McpToolGrantsJsonError::Bounded(PrivateJsonError::Decode(_))
    ));
    let invalid_utf8 = decode_mcp_tool_grants_json(b"\xff").expect_err("invalid UTF-8 is refused");
    assert!(matches!(
        invalid_utf8,
        McpToolGrantsJsonError::Bounded(PrivateJsonError::Decode(_))
    ));
}

#[test]
fn mcp_tool_json_limits() {
    let at_limit = grant_text_of_len(MAX_JSON_BYTES);
    let grants =
        decode_mcp_tool_grants_json(at_limit.as_bytes()).expect("the byte limit is inclusive");
    assert_eq!(grants.len(), 1_usize);
    let mut over_limit = at_limit.into_bytes();
    over_limit.push(b'}');
    let too_large =
        decode_mcp_tool_grants_json(&over_limit).expect_err("a byte over the limit is refused");
    assert!(matches!(
        too_large,
        McpToolGrantsJsonError::Bounded(PrivateJsonError::TooLarge(len, MAX_JSON_BYTES))
            if len == over_limit.len()
    ));
    let at_depth = nested_array_grant(MAX_JSON_DEPTH.saturating_sub(1));
    drop(decode_mcp_tool_grants_json(at_depth.as_bytes()).expect("the depth limit is inclusive"));
    let too_deep = nested_array_grant(MAX_JSON_DEPTH);
    let too_deep_error = decode_mcp_tool_grants_json(too_deep.as_bytes())
        .expect_err("a level over the depth limit is refused");
    assert!(matches!(
        too_deep_error,
        McpToolGrantsJsonError::Bounded(PrivateJsonError::TooDeep(MAX_JSON_DEPTH))
    ));
}

#[rstest]
fn replays_the_mcp_tool_grants_corpus(#[files("fuzz/corpus/mcp_tool_grants/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus seed");
    drop(decode_mcp_tool_grants_json(&bytes));
}
