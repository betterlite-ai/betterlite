#![cfg(test)]

use crate::tool_permission_requests::tests::{
    block_guardrail, forwarded, functions, request, rewrite_guardrail, run, tools,
};

#[test]
fn every_forbidden_tool_is_removed_in_one_request() {
    // Bash and WriteFile match deny rules and Unlisted falls to the default deny
    let mut body = request(&[("tools", &tools(&["Bash", "Read", "WriteFile", "Unlisted"]))]);
    run(&rewrite_guardrail(), &mut body).unwrap();
    assert_eq!(forwarded(body), request(&[("tools", &tools(&["Read"]))]));
}

#[test]
fn forbidden_entries_are_removed_from_all_four_fields_at_once() {
    let mut body = request(&[
        ("tools", &tools(&["Bash", "Read"])),
        ("functions", &functions(&["WriteFile", "Read", "Unlisted"])),
        ("tool_choice", r#""Unlisted""#),
        ("function_call", r#"{"name": "Bash"}"#),
    ]);
    run(&rewrite_guardrail(), &mut body).unwrap();
    assert_eq!(
        forwarded(body),
        request(&[
            ("tools", &tools(&["Read"])),
            ("functions", &functions(&["Read"])),
            ("tool_choice", r#""none""#),
            ("function_call", r#""none""#),
        ])
    );
}

#[test]
fn a_tool_listed_twice_is_removed_both_times() {
    let mut body = request(&[("tools", &tools(&["Bash", "Read", "Bash"]))]);
    run(&rewrite_guardrail(), &mut body).unwrap();
    assert_eq!(forwarded(body), request(&[("tools", &tools(&["Read"]))]));
}

#[test]
fn block_mode_reports_only_the_first_of_several_forbidden_tools() {
    let mut body = request(&[("tools", &tools(&["Bash", "WriteFile"]))]);
    let failure = run(&block_guardrail(), &mut body).unwrap_err().to_string();
    assert!(
        failure.contains("Tool 'Bash' denied"),
        "unexpected failure {failure}"
    );
    assert!(
        !failure.contains("WriteFile"),
        "unexpected failure {failure}"
    );
}
