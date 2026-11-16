#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission_requests::tests::{
    block_guardrail, functions, guardrail, request, run, tools,
};

// The error and detection_message of a 400 guardrail violation, in the order the response lists them
fn violation(failure: GuardrailFailure) -> (Vec<String>, Vec<GuardrailValue>) {
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("expected an HTTP object failure, got {failure:?}");
    };
    assert_eq!(status, 400);
    (
        detail.keys().cloned().collect(),
        detail.into_values().collect(),
    )
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[rstest]
#[case::tools("tools", &tools(&["Bash"]))]
#[case::legacy_functions("functions", &functions(&["Bash"]))]
#[case::forced_tool_choice_name("tool_choice", r#""Bash""#)]
#[case::forced_tool_choice_object(
    "tool_choice",
    r#"{"type": "function", "function": {"name": "Bash"}}"#
)]
#[case::forced_function_call_name("function_call", r#""Bash""#)]
#[case::forced_function_call_object("function_call", r#"{"name": "Bash"}"#)]
fn a_forbidden_tool_in_block_mode_raises_a_400_guardrail_violation(
    #[case] key: &str,
    #[case] value: &str,
) {
    let failure = run(&block_guardrail(), &mut request(&[(key, value)])).unwrap_err();
    assert_eq!(
        violation(failure),
        (
            vec!["error".to_owned(), "detection_message".to_owned()],
            vec![
                text("Violated guardrail policy"),
                text("Tool 'Bash' denied by rule 'deny-bash'"),
            ]
        )
    );
}

#[test]
fn the_message_names_the_default_action_when_no_rule_matched() {
    let mut body = request(&[("tools", &tools(&["Unlisted"]))]);
    let failure = run(&block_guardrail(), &mut body).unwrap_err();
    assert_eq!(
        violation(failure).1[1],
        text("Tool 'Unlisted' denied by default action")
    );
}

#[test]
fn the_first_forbidden_tool_in_request_order_is_reported() {
    let mut body = request(&[("tools", &tools(&["Read", "Write", "Bash"]))]);
    let failure = run(&block_guardrail(), &mut body).unwrap_err();
    assert_eq!(
        violation(failure).1[1],
        text("Tool 'Write' denied by rule 'deny-write'")
    );
}

#[test]
fn a_blocked_request_is_left_as_it_was() {
    let listed = tools(&["Read", "Bash"]);
    let fields = [("tools", listed.as_str()), ("tool_choice", r#""Bash""#)];
    let mut body = request(&fields);
    run(&block_guardrail(), &mut body).unwrap_err();
    assert_eq!(body, request(&fields));
}

#[test]
fn a_default_allow_guardrail_blocks_only_what_a_rule_denies() {
    let checked = guardrail(
        r#"[{"id": "deny-bash", "tool_name": "Bash", "decision": "deny"}]"#,
        ToolDefaultAction::Allow,
        ToolDisallowedAction::Block,
    );
    run(&checked, &mut request(&[("tools", &tools(&["Anything"]))])).unwrap();
    assert!(run(&checked, &mut request(&[("tools", &tools(&["Bash"]))])).is_err());
}

#[test]
fn a_malformed_request_is_a_400_that_names_the_field() {
    let mut body = request(&[("tools", r#""Bash""#)]);
    assert_eq!(
        run(&block_guardrail(), &mut body).unwrap_err(),
        GuardrailFailure::Http {
            status: 400,
            detail: "tools must be a list".to_owned(),
        }
    );
}
