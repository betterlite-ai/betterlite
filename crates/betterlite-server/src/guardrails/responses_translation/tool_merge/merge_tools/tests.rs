#![cfg(test)]

use betterlite_core::api::responses_tool_namespaces::values::Fields;

use super::merge_guardrailed_tools;
use crate::guardrails::responses_translation::tool_merge::tests::support::{
    FLATTENED, NAMESPACE, field_list, fields, json_list,
};

const PLAIN: &str = r#"[{"type": "function", "name": "lookup", "description": "Find", "parameters": {"type": "object"}}]"#;
const PLAIN_FLATTENED: &str = r#"[[{"type": "function", "function": {"name": "lookup", "description": "Find", "parameters": {"type": "object"}}}]]"#;

fn merge(originals: &str, groups: &str, guardrailed: &str) -> Vec<Fields> {
    let flattened: Vec<Vec<Fields>> = serde_json::from_str(groups).unwrap();
    merge_guardrailed_tools(&field_list(originals), &flattened, &json_list(guardrailed)).unwrap()
}

fn namespace_groups() -> String {
    format!("[{FLATTENED}]")
}

#[test]
fn merge_tools_returns_the_originals_when_the_guardrail_changed_nothing() {
    let originals = format!("[{NAMESPACE}]");
    let merged = merge(&originals, &namespace_groups(), FLATTENED);
    assert_eq!(merged, [fields(NAMESPACE)]);
}

#[test]
fn merge_tools_rebuilds_a_namespace_in_place() {
    let originals = format!("[{NAMESPACE}]");
    let guardrailed = FLATTENED.replace("Find a contact", "Find a contact [checked]");
    let merged = merge(&originals, &namespace_groups(), &guardrailed);
    let expected = NAMESPACE.replace("Find a contact", "Find a contact [checked]");
    assert_eq!(merged, [fields(&expected)]);
}

#[test]
fn merge_tools_converts_a_changed_plain_tool_to_the_responses_format() {
    let chat = r#"[{"type": "function", "function": {"name": "lookup", "description": "Find it", "parameters": {"type": "object"}}}]"#;
    let expected = fields(
        r#"{"type": "function", "name": "lookup", "description": "Find it", "parameters": {"type": "object"}, "strict": false}"#,
    );
    assert_eq!(merge(PLAIN, PLAIN_FLATTENED, chat), [expected]);
}

#[test]
fn merge_tools_drops_a_tool_the_guardrail_removed() {
    assert!(merge(PLAIN, PLAIN_FLATTENED, "[]").is_empty());
}

#[test]
fn merge_tools_appends_a_tool_the_guardrail_added() {
    let chat = r#"[{"type": "function", "function": {"name": "lookup", "description": "Find", "parameters": {"type": "object"}}},
        {"type": "function", "function": {"name": "extra", "description": "More"}}]"#;
    let merged = merge(PLAIN, PLAIN_FLATTENED, chat);
    let expected = fields(
        r#"{"type": "function", "name": "extra", "description": "More", "parameters": {"type": "object"}, "strict": false}"#,
    );
    assert_eq!(merged, [field_list(PLAIN).remove(0), expected]);
}

#[test]
fn merge_tools_pairs_duplicate_names_by_their_order() {
    let originals = r#"[{"type": "function", "name": "same", "description": "first"},
        {"type": "function", "name": "same", "description": "second"}]"#;
    let groups = r#"[[{"type": "function", "function": {"name": "same", "description": "first"}}],
        [{"type": "function", "function": {"name": "same", "description": "second"}}]]"#;
    let chat = r#"[{"type": "function", "function": {"name": "same", "description": "first"}},
        {"type": "function", "function": {"name": "same", "description": "changed"}}]"#;
    let merged = merge(originals, groups, chat);
    let expected = fields(
        r#"{"type": "function", "name": "same", "description": "changed", "parameters": {"type": "object"}, "strict": false}"#,
    );
    assert_eq!(merged, [field_list(originals).remove(0), expected]);
}

#[test]
fn merge_tools_ignores_guardrailed_values_that_are_not_objects() {
    let chat = r#"[5, null, {"type": "function", "function": {"name": "lookup", "description": "Find", "parameters": {"type": "object"}}}]"#;
    assert_eq!(merge(PLAIN, PLAIN_FLATTENED, chat), field_list(PLAIN));
}

#[test]
fn merge_tools_keeps_a_tool_that_flattened_to_nothing() {
    let originals = r#"[{"type": "mcp", "server_label": "files"}, {"type": "function", "name": "lookup", "description": "Find", "parameters": {"type": "object"}}]"#;
    let groups = r#"[[], [{"type": "function", "function": {"name": "lookup", "description": "Find", "parameters": {"type": "object"}}}]]"#;
    let chat = r#"[{"type": "function", "function": {"name": "lookup", "description": "Find", "parameters": {"type": "object"}}}]"#;
    assert_eq!(merge(originals, groups, chat), field_list(originals));
}

#[test]
fn merge_tools_converts_a_namespace_whose_members_no_longer_match() {
    let originals = r#"[{"type": "namespace", "name": "crm", "tools": [{"type": "function", "name": "lookup", "description": "Find"}]}]"#;
    let groups = r#"[[{"type": "function", "function": {"name": "crm__lookup", "description": "Find"}},
        {"type": "function", "function": {"name": "crm__other", "description": "Other"}}]]"#;
    let chat = r#"[{"type": "function", "function": {"name": "crm__lookup", "description": "Found"}},
        {"type": "function", "function": {"name": "crm__other", "description": "Other"}}]"#;
    let expected = field_list(
        r#"[{"type": "function", "name": "crm__lookup", "description": "Found", "parameters": {"type": "object"}, "strict": false},
        {"type": "function", "name": "crm__other", "description": "Other", "parameters": {"type": "object"}, "strict": false}]"#,
    );
    assert_eq!(merge(originals, groups, chat), expected);
}

#[test]
fn merge_tools_converts_a_namespace_without_members() {
    let originals = r#"[{"type": "namespace", "name": "crm"}]"#;
    let groups =
        r#"[[{"type": "function", "function": {"name": "crm__lookup", "description": "Find"}}]]"#;
    let chat =
        r#"[{"type": "function", "function": {"name": "crm__lookup", "description": "Found"}}]"#;
    let expected = fields(
        r#"{"type": "function", "name": "crm__lookup", "description": "Found", "parameters": {"type": "object"}, "strict": false}"#,
    );
    assert_eq!(merge(originals, groups, chat), [expected]);
}
