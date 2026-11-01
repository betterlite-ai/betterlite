#![cfg(test)]

use rstest::rstest;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::context::hydrate_tool_context;

fn fields_of(text: &str) -> Fields {
    match Json::parse(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn field_text(fields: &Fields, key: &str) -> Option<String> {
    match Json::field(fields, key) {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[rstest]
#[case::direct(r#"{"mcp_tool_call_metadata": {"name": "search"}}"#)]
#[case::nested_metadata(r#"{"metadata": {"mcp_tool_call_metadata": {"name": "search"}}}"#)]
#[case::nested_litellm_metadata(
    r#"{"litellm_metadata": {"mcp_tool_call_metadata": {"name": "search"}}}"#
)]
fn hydration_populates_name_aliases(#[case] input: &str) {
    let mut data = fields_of(input);
    hydrate_tool_context(&mut data);
    assert_eq!(
        field_text(&data, "mcp_tool_name").as_deref(),
        Some("search")
    );
    assert_eq!(field_text(&data, "tool_name").as_deref(), Some("search"));
    assert_eq!(field_text(&data, "name").as_deref(), Some("search"));
}

#[test]
fn hydration_populates_arguments_and_server_name() {
    let mut data = fields_of(
        r#"{"mcp_tool_call_metadata": {"name": "search", "arguments": {"q": "x"}, "mcp_server_name": "srv"}}"#,
    );
    hydrate_tool_context(&mut data);
    assert_eq!(
        data.get("mcp_arguments").map(|raw| raw.get()),
        data.get("arguments").map(|raw| raw.get())
    );
    assert_eq!(field_text(&data, "mcp_server_name").as_deref(), Some("srv"));
    assert_eq!(field_text(&data, "server_name").as_deref(), Some("srv"));
}

#[test]
fn hydration_keeps_empty_falsy_argument_present() {
    let mut data = fields_of(r#"{"mcp_tool_call_metadata": {"arguments": 0}}"#);
    hydrate_tool_context(&mut data);
    assert!(data.contains_key("mcp_arguments"));
    assert!(data.contains_key("arguments"));
}

#[test]
fn hydration_skips_falsy_name_and_server_name() {
    let mut data = fields_of(r#"{"mcp_tool_call_metadata": {"name": "", "mcp_server_name": ""}}"#);
    hydrate_tool_context(&mut data);
    assert!(!data.contains_key("name"));
    assert!(!data.contains_key("mcp_server_name"));
}

#[test]
fn hydration_skips_absent_arguments() {
    let mut data = fields_of(r#"{"mcp_tool_call_metadata": {"name": "search"}}"#);
    hydrate_tool_context(&mut data);
    assert!(!data.contains_key("mcp_arguments"));
    assert!(!data.contains_key("arguments"));
}

#[test]
fn hydration_never_overwrites_existing_values() {
    let mut data =
        fields_of(r#"{"name": "existing", "mcp_tool_call_metadata": {"name": "search"}}"#);
    hydrate_tool_context(&mut data);
    assert_eq!(field_text(&data, "name").as_deref(), Some("existing"));
    assert_eq!(
        field_text(&data, "mcp_tool_name").as_deref(),
        Some("search")
    );
}

#[test]
fn hydration_is_noop_without_metadata() {
    let mut data = fields_of(r#"{"other": 1}"#);
    hydrate_tool_context(&mut data);
    assert_eq!(
        serde_json::to_string(&data).unwrap_or_default(),
        r#"{"other":1}"#
    );
}

#[test]
fn hydration_ignores_non_object_metadata() {
    let mut data = fields_of(r#"{"mcp_tool_call_metadata": "nope"}"#);
    hydrate_tool_context(&mut data);
    assert!(!data.contains_key("name"));
}
