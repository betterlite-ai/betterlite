#![cfg(test)]

use crate::guardrail_base::failure::GuardrailFailure;
use crate::tool_permission_responses::tests::support::{guardrail, rewrite_guardrail, rule};

use super::{ToolCallInfo, evaluate_tool_calls};

fn call(id: &str, name: &str) -> ToolCallInfo {
    ToolCallInfo {
        id: id.to_owned(),
        kind: Some("function".to_owned()),
        name: Some(name.to_owned()),
        arguments: None,
    }
}

#[test]
fn evaluate_tool_calls_collects_every_denial_in_rewrite_mode() {
    let guardrail = rewrite_guardrail(vec![
        rule("block-shell", "deny", "shell"),
        rule("block-read", "deny", "read"),
        rule("allow-list", "allow", "list"),
    ]);
    let denied = evaluate_tool_calls(
        &guardrail,
        &[call("c1", "shell"), call("c2", "list"), call("c3", "read")],
    )
    .expect("rewrite mode does not raise");
    assert_eq!(denied.len(), 2);
    assert_eq!(denied[0].error.tool_name, "shell");
    assert_eq!(denied[0].error.rule_id.as_deref(), Some("block-shell"));
    assert_eq!(
        denied[0].error.message,
        "Tool 'shell' denied by rule 'block-shell'"
    );
    assert_eq!(denied[1].error.tool_name, "read");
    assert_eq!(denied[0].call.id, "c1");
}

#[test]
fn evaluate_tool_calls_raises_the_first_denial_in_block_mode() {
    let guardrail = guardrail(vec![
        rule("block-shell", "deny", "shell"),
        rule("block-read", "deny", "read"),
    ]);
    let error = evaluate_tool_calls(&guardrail, &[call("c1", "read"), call("c2", "shell")])
        .expect_err("block mode raises");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    } = error
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some("tools-check"));
    assert_eq!(message, "Tool 'read' denied by rule 'block-read'");
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(blocked_content);
}

#[test]
fn evaluate_tool_calls_returns_nothing_when_every_call_is_allowed() {
    let guardrail = guardrail(vec![rule("allow-shell", "allow", "shell")]);
    let denied = evaluate_tool_calls(&guardrail, &[call("c1", "shell")]).expect("allowed");
    assert!(denied.is_empty());
}

#[test]
fn evaluate_tool_calls_denies_a_tool_the_rules_do_not_name_by_default() {
    let guardrail = rewrite_guardrail(Vec::new());
    let denied = evaluate_tool_calls(&guardrail, &[call("c1", "mystery")]).expect("rewrite mode");
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].error.rule_id, None);
    assert_eq!(
        denied[0].error.message,
        "Tool 'mystery' denied by default action"
    );
}

#[test]
fn evaluate_tool_calls_names_a_kind_only_call_by_its_kind() {
    let guardrail = rewrite_guardrail(Vec::new());
    let unnamed = ToolCallInfo {
        id: "c1".to_owned(),
        kind: Some("function".to_owned()),
        name: None,
        arguments: None,
    };
    let denied = evaluate_tool_calls(&guardrail, &[unnamed]).expect("rewrite mode");
    assert_eq!(denied[0].error.tool_name, "unknown_tool");
    assert_eq!(
        denied[0].error.message,
        "Tool 'function' denied by default action"
    );
}

#[test]
fn evaluate_tool_calls_skips_a_call_without_a_name_or_kind() {
    let guardrail = rewrite_guardrail(Vec::new());
    let unnamed = ToolCallInfo {
        id: "c1".to_owned(),
        kind: None,
        name: None,
        arguments: None,
    };
    let denied = evaluate_tool_calls(&guardrail, &[unnamed]).expect("rewrite mode");
    assert!(denied.is_empty());
}

#[test]
fn evaluate_tool_calls_accepts_an_empty_call_list() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    assert!(
        evaluate_tool_calls(&guardrail, &[])
            .expect("nothing to check")
            .is_empty()
    );
}
