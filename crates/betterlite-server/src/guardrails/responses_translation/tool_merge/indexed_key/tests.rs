#![cfg(test)]

use betterlite_core::api::responses_tool_namespaces::error::NamespaceToolsError;
use rstest::rstest;

use super::{chat_tool_key, indexed_keys};
use crate::guardrails::responses_translation::tool_merge::tests::support::{
    field_list, fields, keys_for,
};

#[rstest]
#[case::function_name(r#"{"type": "function", "function": {"name": "a"}}"#, "function:a")]
#[case::function_without_name(r#"{"type": "function", "function": {}}"#, "function:")]
#[case::function_wins_over_name(
    r#"{"type": "function", "function": {"name": "a"}, "name": "b"}"#,
    "function:a"
)]
#[case::server_label(
    r#"{"type": "mcp", "server_label": "files", "name": "n"}"#,
    "mcp:files"
)]
#[case::name_without_label(r#"{"type": "mcp", "server_label": "", "name": "n"}"#, "mcp:n")]
#[case::function_that_is_not_an_object(r#"{"type": "mcp", "function": "x", "name": "n"}"#, "mcp:n")]
#[case::nothing(r#"{"type": "web_search"}"#, "web_search:")]
#[case::empty("{}", ":")]
fn indexed_key_names_a_tool_by_function_server_label_or_name(
    #[case] source: &str,
    #[case] expected: &str,
) {
    assert_eq!(chat_tool_key(&fields(source)).unwrap(), expected);
}

#[rstest]
#[case::type_field(r#"{"type": 5}"#, "type")]
#[case::function_name(r#"{"type": "function", "function": {"name": 5}}"#, "name")]
#[case::server_label(r#"{"type": "mcp", "server_label": []}"#, "server_label")]
#[case::name(r#"{"type": "mcp", "name": {}}"#, "name")]
fn indexed_key_rejects_a_field_that_is_not_text(#[case] source: &str, #[case] field: &str) {
    let error = chat_tool_key(&fields(source)).unwrap_err();
    assert!(matches!(error, NamespaceToolsError::WrongType { field: found, .. } if found == field));
}

#[test]
fn indexed_key_counts_duplicate_names_in_order() {
    let tools = field_list(
        r#"[{"type": "function", "function": {"name": "a"}},
            {"type": "function", "function": {"name": "b"}},
            {"type": "function", "function": {"name": "a"}},
            {"type": "function", "function": {"name": "a"}}]"#,
    );
    let expected = [
        ("function:a".to_owned(), 0),
        ("function:b".to_owned(), 0),
        ("function:a".to_owned(), 1),
        ("function:a".to_owned(), 2),
    ];
    assert_eq!(keys_for(&tools), expected);
}

#[test]
fn indexed_key_is_empty_for_no_tools() {
    assert!(indexed_keys(&[]).unwrap().is_empty());
}

#[test]
fn indexed_key_fails_on_the_first_tool_with_a_bad_field() {
    let tools = field_list(r#"[{"type": "function"}, {"type": 1}]"#);
    let references: Vec<_> = tools.iter().collect();
    let error = indexed_keys(&references).unwrap_err();
    assert!(matches!(
        error,
        NamespaceToolsError::WrongType { field: "type", .. }
    ));
}
