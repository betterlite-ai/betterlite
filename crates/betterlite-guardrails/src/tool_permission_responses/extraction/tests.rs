#![cfg(test)]

use super::{extract_response_tools, legacy_to_tool_call};
use crate::tool_permission_responses::tests::support::{json, object, text};

#[test]
fn extract_response_tools_reads_a_modern_tool_call() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]}}]}"#,
    );
    let calls = extract_response_tools(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(calls[0].name.as_deref(), Some("read"));
    assert_eq!(calls[0].arguments, Some(text(r#"{"path":"a"}"#)));
}

#[test]
fn extract_response_tools_gives_a_legacy_call_the_choice_position_as_its_id() {
    let response = object(
        r#"{"choices":[{"message":{}},{"message":{"function_call":{"name":"shell","arguments":"{}"}}}]}"#,
    );
    let calls = extract_response_tools(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "legacy_function_call_1");
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(calls[0].name.as_deref(), Some("shell"));
    assert_eq!(calls[0].arguments, Some(text("{}")));
}

#[test]
fn extract_response_tools_keeps_modern_calls_before_legacy_ones() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"call_1","function":{"name":"read"}}],"function_call":{"name":"shell"}}}]}"#,
    );
    let calls = extract_response_tools(&response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[1].id, "legacy_function_call_0");
}

#[test]
fn legacy_to_tool_call_defaults_missing_arguments_to_an_empty_string() {
    let call = legacy_to_tool_call(&json(r#"{"name":"shell"}"#), 3).expect("legacy call");
    assert_eq!(call.id, "legacy_function_call_3");
    assert_eq!(call.arguments, Some(text("")));
}

#[test]
fn legacy_to_tool_call_keeps_arguments_of_every_declared_shape() {
    let null = legacy_to_tool_call(&json(r#"{"name":"shell","arguments":null}"#), 0).expect("call");
    assert_eq!(null.arguments, Some(text("")));
    let mapping = legacy_to_tool_call(&json(r#"{"name":"shell","arguments":{"cmd":"ls"}}"#), 0)
        .expect("call");
    assert_eq!(mapping.arguments, Some(json(r#"{"cmd":"ls"}"#)));
}

#[test]
fn legacy_to_tool_call_needs_a_non_empty_name() {
    assert!(legacy_to_tool_call(&json(r#"{"arguments":"{}"}"#), 0).is_none());
    assert!(legacy_to_tool_call(&json(r#"{"name":"","arguments":"{}"}"#), 0).is_none());
    assert!(legacy_to_tool_call(&json(r#"{"name":5}"#), 0).is_none());
}

#[test]
fn extract_response_tools_ignores_entries_that_are_not_objects() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[7,{"id":"call_1","function":{"name":"read"}}]}}]}"#,
    );
    let calls = extract_response_tools(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
}

#[test]
fn extract_response_tools_handles_missing_choices_and_messages() {
    assert!(extract_response_tools(&object("{}")).is_empty());
    assert!(extract_response_tools(&object(r#"{"choices":[{"message":"nope"}]}"#)).is_empty());
    assert!(extract_response_tools(&object(r#"{"choices":"nope"}"#)).is_empty());
}

#[test]
fn extract_response_tools_keeps_a_missing_kind_and_arguments() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"call_1","function":{"name":"read"}}]}}]}"#,
    );
    let calls = extract_response_tools(&response);
    assert_eq!(calls[0].kind, None);
    assert_eq!(calls[0].arguments, None);
}

#[test]
fn extract_response_tools_uses_an_empty_id_for_a_non_text_id() {
    let response =
        object(r#"{"choices":[{"message":{"tool_calls":[{"id":5,"function":{"name":"read"}}]}}]}"#);
    assert_eq!(extract_response_tools(&response)[0].id, "");
}
