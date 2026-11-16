#![cfg(test)]

use rstest::rstest;

use super::{as_tool, namespace_members, validated_tools};
use crate::guardrails::responses_translation::tool_merge::tests::support::{fields, json_list};

#[test]
fn as_tool_returns_an_object_as_the_tool() {
    let value = json_list(r#"[{"type": "function"}]"#).remove(0);
    assert_eq!(as_tool(&value), Some(&fields(r#"{"type": "function"}"#)));
}

#[rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::number("5")]
#[case::text(r#""function""#)]
#[case::list(r#"[{"type": "function"}]"#)]
fn as_tool_rejects_a_value_that_is_not_an_object(#[case] source: &str) {
    let value = json_list(&format!("[{source}]")).remove(0);
    assert_eq!(as_tool(&value), None);
}

#[test]
fn validation_drops_the_tools_that_are_not_objects() {
    let values = json_list(r#"[{"name": "a"}, 5, null, {"name": "b"}]"#);
    let tools = validated_tools(&values);
    assert_eq!(
        tools,
        [&fields(r#"{"name": "a"}"#), &fields(r#"{"name": "b"}"#)]
    );
}

#[test]
fn validation_keeps_every_tool_when_all_are_objects() {
    let values = json_list(r#"[{"name": "a"}]"#);
    assert_eq!(validated_tools(&values).len(), 1);
}

#[test]
fn namespace_members_skip_a_member_that_is_not_an_object() {
    let namespace = fields(r#"{"tools": [{"name": "a"}, "b", {"name": "c"}]}"#);
    assert_eq!(
        namespace_members(&namespace),
        [&fields(r#"{"name": "a"}"#), &fields(r#"{"name": "c"}"#)]
    );
}

#[rstest]
#[case::no_list(r#"{"name": "n"}"#)]
#[case::text_list(r#"{"tools": "abc"}"#)]
#[case::null_list(r#"{"tools": null}"#)]
fn namespace_members_are_empty_without_a_list(#[case] source: &str) {
    assert!(namespace_members(&fields(source)).is_empty());
}
