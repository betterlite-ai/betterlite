#![cfg(test)]

use rstest::rstest;

use super::{ToolVerdict, check_tool_permission};
use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission_requests::tests::{RULES, args, guardrail, guardrail_from};

#[rstest]
#[case::denied_by_rule(
    "Bash",
    false,
    Some("deny-bash"),
    "Tool 'Bash' denied by rule 'deny-bash'"
)]
#[case::allowed_by_rule(
    "Read",
    true,
    Some("allow-read"),
    "Tool 'Read' allowed by rule 'allow-read'"
)]
#[case::pattern_rule(
    "WriteFile",
    false,
    Some("deny-write"),
    "Tool 'WriteFile' denied by rule 'deny-write'"
)]
#[case::default_deny("Other", false, None, "Tool 'Other' denied by default action")]
fn a_tool_takes_the_decision_of_its_first_matching_rule_or_the_default(
    #[case] tool: &str,
    #[case] is_allowed: bool,
    #[case] rule_id: Option<&str>,
    #[case] message: &str,
) {
    let checked = guardrail(RULES, ToolDefaultAction::Deny, ToolDisallowedAction::Block);
    assert_eq!(
        check_tool_permission(&checked, tool),
        ToolVerdict {
            is_allowed,
            rule_id: rule_id.map(str::to_owned),
            message: message.to_owned(),
        }
    );
}

#[test]
fn a_tool_without_a_matching_rule_is_allowed_under_a_default_allow() {
    let checked = guardrail(RULES, ToolDefaultAction::Allow, ToolDisallowedAction::Block);
    assert_eq!(
        check_tool_permission(&checked, "Other"),
        ToolVerdict {
            is_allowed: true,
            rule_id: None,
            message: "Tool 'Other' allowed by default action".to_owned(),
        }
    );
}

#[test]
fn rules_are_checked_in_order_and_the_first_match_wins() {
    let ordered = r#"[
        {"id": "allow-all", "tool_name": ".*", "decision": "allow"},
        {"id": "deny-bash", "tool_name": "Bash", "decision": "deny"}
    ]"#;
    let checked = guardrail(
        ordered,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Block,
    );
    let verdict = check_tool_permission(&checked, "Bash");
    assert!(verdict.is_allowed);
    assert_eq!(verdict.rule_id.as_deref(), Some("allow-all"));
}

#[test]
fn a_name_pattern_must_match_the_whole_tool_name() {
    let checked = guardrail(RULES, ToolDefaultAction::Allow, ToolDisallowedAction::Block);
    let verdict = check_tool_permission(&checked, "BashRunner");
    assert!(verdict.is_allowed);
    assert_eq!(verdict.rule_id, None);
}

#[rstest]
#[case::function_type_matches("function", Some("deny-typed"))]
#[case::other_type_does_not_match("mcp", None)]
fn a_tool_type_rule_sees_every_request_tool_as_a_function(
    #[case] rule_type: &str,
    #[case] rule_id: Option<&str>,
) {
    let typed =
        format!(r#"[{{"id": "deny-typed", "tool_type": "{rule_type}", "decision": "deny"}}]"#);
    let checked = guardrail(
        &typed,
        ToolDefaultAction::Allow,
        ToolDisallowedAction::Block,
    );
    assert_eq!(
        check_tool_permission(&checked, "Anything")
            .rule_id
            .as_deref(),
        rule_id
    );
}

#[test]
fn a_rule_with_argument_patterns_decides_without_any_argument() {
    let patterned = r#"[
        {"id": "allow-ls", "tool_name": "Bash", "decision": "allow", "allowed_param_patterns": {"command": "^ls$"}}
    ]"#;
    let checked = guardrail(
        patterned,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Block,
    );
    let verdict = check_tool_permission(&checked, "Bash");
    assert!(verdict.is_allowed);
    assert_eq!(verdict.rule_id.as_deref(), Some("allow-ls"));
}

#[rstest]
#[case::rule("Bash", "Bash|deny-bash|Tool 'Bash' denied by rule 'deny-bash'")]
#[case::default("Other", "Other|None|Tool 'Other' denied by default action")]
fn the_violation_message_template_sees_the_tool_the_rule_and_the_default(
    #[case] tool: &str,
    #[case] expected: &str,
) {
    let mut templated = args(RULES, ToolDefaultAction::Deny, ToolDisallowedAction::Block);
    templated.violation_message_template =
        Some("{tool_name}|{rule_id}|{default_message}".to_owned());
    assert_eq!(
        check_tool_permission(&guardrail_from(templated), tool).message,
        expected
    );
}
