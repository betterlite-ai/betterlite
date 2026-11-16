#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission_requests::pre_call_hook;
use crate::tool_permission_requests::tests::{
    NAME, RULES, args, block_guardrail, guardrail_from, request, rewrite_guardrail, run, tools,
};

#[rstest]
#[case::empty_request(&[("model", r#""gpt-4o""#)])]
#[case::null_lists(&[("tools", "null"), ("functions", "null"), ("tool_choice", "null"), ("function_call", "null")])]
#[case::empty_lists(&[("tools", "[]"), ("functions", "[]")])]
#[case::only_unforced_choices(&[("tool_choice", r#""auto""#), ("function_call", r#""none""#)])]
#[case::required_choice(&[("tool_choice", r#""required""#)])]
#[case::unnamed_tools_only(&[("tools", r#"[{"type": "web_search"}, {"type": "function"}]"#)])]
fn a_request_without_named_tools_is_returned_unchanged(#[case] fields: &[(&str, &str)]) {
    for checked in [block_guardrail(), rewrite_guardrail()] {
        let mut body = request(fields);
        run(&checked, &mut body).unwrap();
        assert_eq!(body, request(fields));
    }
}

#[test]
fn a_guardrail_for_another_event_returns_a_forbidden_request_unchanged() {
    let sent = tools(&["Bash"]);
    let fields = [("tools", sent.as_str()), ("tool_choice", r#""Bash""#)];
    let mut post_call = args(
        RULES,
        ToolDefaultAction::Deny,
        ToolDisallowedAction::Rewrite,
    );
    post_call.event_hook = LitellmParamsMode::One("post_call".to_owned());
    let mut body = request(&fields);
    run(&guardrail_from(post_call), &mut body).unwrap();
    assert_eq!(body, request(&fields));
}

#[test]
fn a_guardrail_that_is_off_by_default_waits_for_the_request_to_name_it() {
    let sent = tools(&["Bash"]);
    let mut off = args(RULES, ToolDefaultAction::Deny, ToolDisallowedAction::Block);
    off.default_on = false;
    let checked = guardrail_from(off);
    let mut unrequested = request(&[("tools", &sent)]);
    run(&checked, &mut unrequested).unwrap();
    assert_eq!(unrequested, request(&[("tools", &sent)]));
    let named = format!(r#"["{NAME}"]"#);
    let mut requested = request(&[("tools", &sent), ("guardrails", &named)]);
    assert!(run(&checked, &mut requested).is_err());
}

#[test]
fn a_guardrail_an_auto_router_turned_off_is_skipped() {
    let sent = tools(&["Bash"]);
    let suppressed = BTreeSet::from([NAME.to_owned()]);
    let mut body = request(&[("tools", &sent)]);
    pre_call_hook(&block_guardrail(), &mut body, &suppressed).unwrap();
    assert_eq!(body, request(&[("tools", &sent)]));
    assert!(run(&block_guardrail(), &mut request(&[("tools", &sent)])).is_err());
}

#[test]
fn a_global_guardrail_the_request_opts_out_of_is_skipped() {
    let sent = tools(&["Bash"]);
    let metadata =
        format!(r#"{{"user_api_key_metadata": {{"opted_out_global_guardrails": ["{NAME}"]}}}}"#);
    let fields = [("tools", sent.as_str()), ("metadata", metadata.as_str())];
    let mut body = request(&fields);
    run(&block_guardrail(), &mut body).unwrap();
    assert_eq!(body, request(&fields));
}

#[test]
fn a_global_guardrail_the_key_disables_is_skipped() {
    let sent = tools(&["Bash"]);
    let metadata = r#"{"user_api_key_metadata": {"disable_global_guardrails": true}}"#;
    let fields = [("tools", sent.as_str()), ("metadata", metadata)];
    let mut body = request(&fields);
    run(&block_guardrail(), &mut body).unwrap();
    assert_eq!(body, request(&fields));
}

#[test]
fn a_malformed_selection_field_fails_the_request() {
    let mut off = args(RULES, ToolDefaultAction::Deny, ToolDisallowedAction::Block);
    off.default_on = false;
    let mut body = request(&[("tools", &tools(&["Bash"])), ("guardrails", r#""x""#)]);
    assert!(run(&guardrail_from(off), &mut body).is_err());
}
