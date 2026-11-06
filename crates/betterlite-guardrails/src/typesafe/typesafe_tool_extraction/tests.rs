#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{ToolCallEntry, tool_call_entries, tool_call_entry};
use crate::typesafe::fixtures::json;

fn entry(name: Option<Json>, arguments: Option<Json>) -> ToolCallEntry {
    ToolCallEntry { name, arguments }
}

#[test]
fn tool_call_entry_reads_the_openai_function_layout() {
    let call = json(r#"{"function":{"name":"lookup","arguments":{"q":1}}}"#);
    assert_eq!(
        tool_call_entry(&call),
        Some(entry(Some(json(r#""lookup""#)), Some(json(r#"{"q":1}"#))))
    );
}

#[test]
fn tool_call_entry_reads_a_direct_name_and_arguments_layout() {
    let call = json(r#"{"name":"lookup","arguments":"q=1"}"#);
    assert_eq!(
        tool_call_entry(&call),
        Some(entry(Some(json(r#""lookup""#)), Some(json(r#""q=1""#))))
    );
}

#[test]
fn tool_call_entry_falls_back_when_function_is_not_an_object() {
    let call = json(r#"{"function":"broken","name":"lookup","arguments":"q=1"}"#);
    assert_eq!(
        tool_call_entry(&call),
        Some(entry(Some(json(r#""lookup""#)), Some(json(r#""q=1""#))))
    );
}

#[test]
fn tool_call_entry_keeps_missing_fields_as_null() {
    let call = json(r#"{"function":{"name":"lookup"}}"#);
    let parsed = tool_call_entry(&call).expect("an entry");
    assert_eq!(parsed.name, Some(json(r#""lookup""#)));
    assert_eq!(parsed.arguments, None);
    assert_eq!(
        serde_json::to_string(&parsed).expect("serialize"),
        r#"{"name":"lookup","arguments":null}"#
    );
}

#[rstest]
#[case::string(r#""lookup""#)]
#[case::list(r#"[{"name":"lookup"}]"#)]
#[case::null("null")]
fn tool_call_entry_rejects_non_objects(#[case] text: &str) {
    assert!(tool_call_entry(&json(text)).is_none());
}

#[test]
fn tool_call_entries_keeps_valid_entries_in_order() {
    let message = json(
        r#"{"role":"assistant","tool_calls":[{"function":{"name":"first"}},"broken",{"name":"second","arguments":"a"}]}"#,
    );
    let entries = tool_call_entries(&message);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, Some(json(r#""first""#)));
    assert_eq!(entries[1].name, Some(json(r#""second""#)));
    assert_eq!(entries[1].arguments, Some(json(r#""a""#)));
}

#[rstest]
#[case::missing(r#"{"role":"assistant"}"#)]
#[case::not_a_list(r#"{"role":"assistant","tool_calls":{"name":"lookup"}}"#)]
#[case::null(r#"{"role":"assistant","tool_calls":null}"#)]
fn tool_call_entries_is_empty_without_a_tool_call_list(#[case] text: &str) {
    assert!(tool_call_entries(&json(text)).is_empty());
}
