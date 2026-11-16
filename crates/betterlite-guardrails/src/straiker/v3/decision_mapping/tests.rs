#![cfg(test)]

use rstest::rstest;

use super::decision;
use crate::straiker::v3::fixtures::{as_json, object, parsed};

#[rstest]
#[case::hook_decision_with_nested_verdict(r#"{"hookSpecificOutput":{"permissionDecision":"DENY"},"straiker":{"action":"allow","blocked_by":["c1"]}}"#, Some("deny"), r#"{"action":"allow","blocked_by":["c1"]}"#)]
#[case::hook_decision_beats_action(
    r#"{"hookSpecificOutput":{"permissionDecision":"Block"},"action":"NONE"}"#,
    Some("block"),
    r#"{"hookSpecificOutput":{"permissionDecision":"Block"},"action":"NONE"}"#
)]
#[case::flat_action(r#"{"action":"BLOCKED"}"#, Some("blocked"), r#"{"action":"BLOCKED"}"#)]
#[case::nested_action(r#"{"straiker":{"action":"Ask"}}"#, Some("ask"), r#"{"action":"Ask"}"#)]
#[case::empty_permission_falls_to_action(
    r#"{"hookSpecificOutput":{"permissionDecision":""},"action":"ALLOW"}"#,
    Some("allow"),
    r#"{"hookSpecificOutput":{"permissionDecision":""},"action":"ALLOW"}"#
)]
#[case::non_text_permission_falls_to_action(
    r#"{"hookSpecificOutput":{"permissionDecision":1},"action":"deny"}"#,
    Some("deny"),
    r#"{"hookSpecificOutput":{"permissionDecision":1},"action":"deny"}"#
)]
#[case::hook_that_is_not_an_object(
    r#"{"hookSpecificOutput":"deny","action":"allow"}"#,
    Some("allow"),
    r#"{"hookSpecificOutput":"deny","action":"allow"}"#
)]
#[case::empty_action_is_undecided(r#"{"action":""}"#, None, r#"{"action":""}"#)]
#[case::nested_verdict_that_is_not_an_object(
    r#"{"straiker":"x","action":"deny"}"#,
    Some("deny"),
    r#"{"straiker":"x","action":"deny"}"#
)]
#[case::no_verdict("{}", None, "{}")]
fn reads_decision(#[case] source: &str, #[case] expected: Option<&str>, #[case] verdict: &str) {
    let body = object(source);
    let (stated, found) = decision(&body);
    assert_eq!(stated.as_deref(), expected);
    assert_eq!(as_json(found), parsed(verdict));
}
