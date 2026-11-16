#![cfg(test)]

use rstest::rstest;

use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission_requests::tests::{
    APPLIED, RULES, forwarded, functions, guardrail, request, run, tools,
};

const CHOICE: &str = r#"{"type": "function", "function": {"name": "Read"}}"#;
const MESSAGES: &str = r#"[{"role": "user", "content": "hi"}]"#;

#[rstest]
#[case::block(ToolDisallowedAction::Block)]
#[case::rewrite(ToolDisallowedAction::Rewrite)]
fn a_request_with_only_allowed_tools_passes_through_unchanged(
    #[case] on_disallowed_action: ToolDisallowedAction,
) {
    let listed_tools =
        r#"[{"type": "function", "function": {"name": "Read"}}, {"type": "web_search"}]"#;
    let listed_functions = functions(&["Read"]);
    let fields = [
        ("model", r#""gpt-4o""#),
        ("messages", MESSAGES),
        ("tools", listed_tools),
        ("functions", listed_functions.as_str()),
        ("tool_choice", CHOICE),
        ("function_call", r#""Read""#),
    ];
    let checked = guardrail(RULES, ToolDefaultAction::Deny, on_disallowed_action);
    let mut body = request(&fields);
    run(&checked, &mut body).unwrap();
    // the applied-guardrail header is the one thing the hook adds
    assert_eq!(
        body.get("metadata"),
        request(&[("metadata", APPLIED)]).get("metadata")
    );
    assert_eq!(forwarded(body), request(&fields));
}

#[rstest]
#[case::block(ToolDisallowedAction::Block)]
#[case::rewrite(ToolDisallowedAction::Rewrite)]
fn a_default_allow_guardrail_lets_unlisted_tools_through(
    #[case] on_disallowed_action: ToolDisallowedAction,
) {
    let checked = guardrail("[]", ToolDefaultAction::Allow, on_disallowed_action);
    let listed = tools(&["Anything"]);
    let pairs = [("tools", listed.as_str()), ("tool_choice", r#""Anything""#)];
    let mut body = request(&pairs);
    run(&checked, &mut body).unwrap();
    assert_eq!(forwarded(body), request(&pairs));
}

#[test]
fn an_allow_rule_listed_before_a_deny_rule_keeps_the_tool() {
    let ordered = r#"[
        {"id": "allow-bash", "tool_name": "Bash", "decision": "allow"},
        {"id": "deny-all", "tool_name": ".*", "decision": "deny"}
    ]"#;
    let checked = guardrail(
        ordered,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Block,
    );
    let sent = tools(&["Bash"]);
    let mut body = request(&[("tools", &sent)]);
    run(&checked, &mut body).unwrap();
    assert_eq!(forwarded(body), request(&[("tools", &sent)]));
}
