#![cfg(test)]

use super::rebuilt_member;
use crate::guardrails::responses_translation::tool_merge::tests::support::{
    FLATTENED, NAMESPACE, fields, rebuild,
};

const LOOKUP_ONLY: &str = r#"{"type": "namespace", "name": "crm", "tools": [
    {"type": "function", "name": "lookup", "description": "Find a contact"}]}"#;

const LOOKUP_FLATTENED: &str = r#"[{"type": "function", "function": {"name": "crm__lookup", "description": "Find a contact", "parameters": {"type": "object"}}}]"#;

const NOTE_FLATTENED: &str = r#"[{"type": "function", "function": {"name": "crm__note", "description": "CRM tools\n\nWrite a note\n\nFormat:\n```lark\nstart: WORD\n```", "parameters": {"type": "object"}}}]"#;

#[test]
fn rebuild_namespace_strips_the_namespace_prefix_and_the_grammar_suffix() {
    let guardrailed = FLATTENED
        .replace("Find a contact", "Find a contact [checked]")
        .replace("Write a note", "Write a note [checked]");
    let expected = fields(
        r#"{"type": "namespace", "name": "crm", "description": "CRM tools", "tools": [
        {"type": "function", "name": "lookup", "description": "Find a contact [checked]"},
        {"type": "custom", "name": "note", "description": "Write a note [checked]",
         "format": {"type": "grammar", "syntax": "lark", "definition": "start: WORD"}},
        {"type": "web_search"}]}"#,
    );
    assert_eq!(rebuild(NAMESPACE, FLATTENED, &guardrailed), [expected]);
}

#[test]
fn rebuild_namespace_leaves_a_member_the_guardrail_did_not_change() {
    let guardrailed = FLATTENED.replace("Write a note", "Write a note [checked]");
    let expected = fields(
        r#"{"type": "namespace", "name": "crm", "description": "CRM tools", "tools": [
        {"type": "function", "name": "lookup", "description": "Find a contact"},
        {"type": "custom", "name": "note", "description": "Write a note [checked]",
         "format": {"type": "grammar", "syntax": "lark", "definition": "start: WORD"}},
        {"type": "web_search"}]}"#,
    );
    assert_eq!(rebuild(NAMESPACE, FLATTENED, &guardrailed), [expected]);
}

#[test]
fn rebuild_namespace_drops_a_member_the_guardrail_removed() {
    let expected = fields(
        r#"{"type": "namespace", "name": "crm", "description": "CRM tools", "tools": [
        {"type": "custom", "name": "note", "description": "Write a note",
         "format": {"type": "grammar", "syntax": "lark", "definition": "start: WORD"}},
        {"type": "web_search"}]}"#,
    );
    assert_eq!(rebuild(NAMESPACE, FLATTENED, NOTE_FLATTENED), [expected]);
}

#[test]
fn rebuild_namespace_is_removed_when_no_member_is_left() {
    assert!(rebuild(LOOKUP_ONLY, LOOKUP_FLATTENED, "[]").is_empty());
}

#[test]
fn rebuild_namespace_keeps_a_description_that_has_no_namespace_prefix() {
    let guardrailed = LOOKUP_FLATTENED.replace("Find a contact", r"\n\nFind a contact");
    let expected = fields(
        r#"{"type": "namespace", "name": "crm", "tools": [
        {"type": "function", "name": "lookup", "description": "\n\nFind a contact"}]}"#,
    );
    assert_eq!(
        rebuild(LOOKUP_ONLY, LOOKUP_FLATTENED, &guardrailed),
        [expected]
    );
}

#[test]
fn rebuild_namespace_writes_a_changed_extra_and_a_changed_function_field() {
    let guardrailed = r#"[{"type": "function", "function": {"name": "crm__lookup", "description": "Find a contact", "parameters": {"type": "object", "x": 1}}, "defer_loading": true}]"#;
    let expected = fields(
        r#"{"type": "namespace", "name": "crm", "tools": [
        {"type": "function", "name": "lookup", "description": "Find a contact",
         "defer_loading": true, "parameters": {"type": "object", "x": 1}}]}"#,
    );
    assert_eq!(
        rebuild(LOOKUP_ONLY, LOOKUP_FLATTENED, guardrailed),
        [expected]
    );
}

#[test]
fn rebuild_namespace_treats_a_null_value_like_an_absent_key() {
    let member = fields(r#"{"type": "function", "name": "lookup"}"#);
    let flattened = fields(r#"{"type": "function", "function": {"name": "lookup"}}"#);
    let guardrailed = fields(
        r#"{"type": "function", "function": {"name": "lookup", "strict": null}, "cache_control": null}"#,
    );
    let rebuilt = rebuilt_member(&member, &flattened, &guardrailed, "").unwrap();
    assert_eq!(rebuilt, member);
}
