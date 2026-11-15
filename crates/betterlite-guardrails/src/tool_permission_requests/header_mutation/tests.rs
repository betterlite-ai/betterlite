#![cfg(test)]

use rstest::rstest;

use super::record_applied_guardrail;
use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission_requests::tests::{
    APPLIED, RULES, args, block_guardrail, guardrail_from, request, rewrite_guardrail, run, tools,
};

#[rstest]
#[case::no_bucket(&[], &[("metadata", r#"{"applied_guardrails": ["g"]}"#)])]
#[case::empty_bucket(&[("metadata", "{}")], &[("metadata", r#"{"applied_guardrails": ["g"]}"#)])]
#[case::other_keys_kept(&[("metadata", r#"{"user": "u"}"#)], &[("metadata", r#"{"user": "u", "applied_guardrails": ["g"]}"#)])]
#[case::existing_names_kept(&[("metadata", r#"{"applied_guardrails": ["other"]}"#)], &[("metadata", r#"{"applied_guardrails": ["other", "g"]}"#)])]
#[case::already_listed(&[("metadata", r#"{"applied_guardrails": ["g"]}"#)], &[("metadata", r#"{"applied_guardrails": ["g"]}"#)])]
#[case::proxy_bucket_wins(&[("metadata", r#"{"a": 1}"#), ("litellm_metadata", r#"{"b": 2}"#)], &[("metadata", r#"{"a": 1}"#), ("litellm_metadata", r#"{"b": 2, "applied_guardrails": ["g"]}"#)])]
#[case::null_bucket_replaced(&[("metadata", "null")], &[("metadata", r#"{"applied_guardrails": ["g"]}"#)])]
#[case::null_proxy_bucket_replaced(&[("litellm_metadata", "null")], &[("litellm_metadata", r#"{"applied_guardrails": ["g"]}"#)])]
fn the_guardrail_name_joins_the_applied_list_once(
    #[case] sent: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let mut body = request(sent);
    record_applied_guardrail(&mut body, "g");
    assert_eq!(body, request(expected));
}

#[test]
fn an_allowed_request_gets_the_header() {
    let mut body = request(&[("tools", &tools(&["Read"]))]);
    run(&block_guardrail(), &mut body).unwrap();
    assert_eq!(
        body,
        request(&[("tools", &tools(&["Read"])), ("metadata", APPLIED),])
    );
}

#[test]
fn a_rewritten_request_gets_the_header_after_the_rewrite() {
    let mut body = request(&[("tools", &tools(&["Read", "Bash"]))]);
    run(&rewrite_guardrail(), &mut body).unwrap();
    assert_eq!(
        body,
        request(&[("tools", &tools(&["Read"])), ("metadata", APPLIED),])
    );
}

#[test]
fn a_rewrite_with_nothing_forbidden_still_gets_the_header() {
    let mut body = request(&[("tool_choice", r#""Read""#)]);
    run(&rewrite_guardrail(), &mut body).unwrap();
    assert_eq!(
        body,
        request(&[("tool_choice", r#""Read""#), ("metadata", APPLIED),])
    );
}

#[test]
fn running_twice_lists_the_guardrail_once() {
    let mut body = request(&[("tools", &tools(&["Read"]))]);
    run(&block_guardrail(), &mut body).unwrap();
    run(&block_guardrail(), &mut body).unwrap();
    assert_eq!(
        body.get("metadata"),
        request(&[("metadata", APPLIED)]).get("metadata")
    );
}

#[test]
fn a_blocked_request_gets_no_header() {
    let mut body = request(&[("tools", &tools(&["Bash"]))]);
    run(&block_guardrail(), &mut body).unwrap_err();
    assert!(!body.contains_key("metadata"));
}

#[test]
fn a_request_without_tools_gets_no_header() {
    let mut body = request(&[("messages", "[]")]);
    run(&block_guardrail(), &mut body).unwrap();
    assert!(!body.contains_key("metadata"));
}

#[test]
fn a_guardrail_without_a_name_adds_no_header() {
    let mut unnamed = args(
        RULES,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Rewrite,
    );
    unnamed.name = String::new();
    let mut body = request(&[("tools", &tools(&["Read"]))]);
    run(&guardrail_from(unnamed), &mut body).unwrap();
    assert!(!body.contains_key("metadata"));
}
