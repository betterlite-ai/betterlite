#![cfg(test)]

use rstest::rstest;

use crate::tool_permission_requests::tests::{
    Data, forwarded, functions, request, rewrite_guardrail, run, tools,
};

fn rewritten(fields: &[(&str, &str)]) -> Data {
    let mut body = request(fields);
    run(&rewrite_guardrail(), &mut body).unwrap();
    forwarded(body)
}

#[test]
fn a_forbidden_tool_is_removed_from_tools_and_the_rest_keep_their_order() {
    // Bash is denied by a rule and Other by the default action, so only Read stays
    let sent = tools(&["Read", "Bash", "Other"]);
    assert_eq!(
        rewritten(&[("model", r#""gpt-4o""#), ("tools", &sent)]),
        request(&[("model", r#""gpt-4o""#), ("tools", &tools(&["Read"]))])
    );
}

#[test]
fn a_forbidden_function_is_removed_from_functions() {
    assert_eq!(
        rewritten(&[("functions", &functions(&["Read", "Bash"]))]),
        request(&[("functions", &functions(&["Read"]))])
    );
}

#[rstest]
#[case::tool_choice_name("tool_choice", r#""Bash""#)]
#[case::tool_choice_object("tool_choice", r#"{"type": "function", "function": {"name": "Bash"}}"#)]
#[case::function_call_name("function_call", r#""Bash""#)]
#[case::function_call_object("function_call", r#"{"name": "Bash"}"#)]
fn a_forbidden_forced_choice_becomes_none(#[case] key: &str, #[case] value: &str) {
    assert_eq!(rewritten(&[(key, value)]), request(&[(key, r#""none""#)]));
}

#[rstest]
#[case::tool_choice("tool_choice", r#""Read""#)]
#[case::tool_choice_object("tool_choice", r#"{"type": "function", "function": {"name": "Read"}}"#)]
#[case::function_call("function_call", r#""Read""#)]
#[case::function_call_object("function_call", r#"{"name": "Read"}"#)]
fn an_allowed_forced_choice_is_kept(#[case] key: &str, #[case] value: &str) {
    assert_eq!(
        rewritten(&[(key, value), ("tools", &tools(&["Bash"]))]),
        request(&[(key, value), ("tools", "[]")])
    );
}

#[test]
fn removing_every_tool_leaves_an_empty_list() {
    assert_eq!(
        rewritten(&[
            ("tools", &tools(&["Bash"])),
            ("functions", &functions(&["Bash"])),
        ]),
        request(&[("tools", "[]"), ("functions", "[]")])
    );
}

#[test]
fn a_list_the_request_never_had_is_not_added() {
    assert_eq!(
        rewritten(&[("function_call", r#""Bash""#)]),
        request(&[("function_call", r#""none""#)])
    );
}

#[test]
fn a_tool_that_is_not_a_function_tool_is_kept() {
    let sent = r#"[{"type": "web_search"}, {"type": "mcp", "function": {"name": "Bash"}}, {"type": "function", "function": {"name": "Bash"}}]"#;
    let kept = r#"[{"type": "web_search"}, {"type": "mcp", "function": {"name": "Bash"}}]"#;
    assert_eq!(rewritten(&[("tools", sent)]), request(&[("tools", kept)]));
}

#[test]
fn a_forbidden_name_is_removed_from_every_place_it_appears() {
    assert_eq!(
        rewritten(&[
            ("tools", &tools(&["Bash", "Read"])),
            ("functions", &functions(&["Bash"])),
            ("tool_choice", r#""Bash""#),
            ("function_call", r#"{"name": "Bash"}"#),
        ]),
        request(&[
            ("tools", &tools(&["Read"])),
            ("functions", "[]"),
            ("tool_choice", r#""none""#),
            ("function_call", r#""none""#),
        ])
    );
}

#[test]
fn a_forbidden_forced_choice_also_removes_the_tool_of_that_name() {
    assert_eq!(
        rewritten(&[
            ("tools", &tools(&["Read", "Bash"])),
            ("tool_choice", r#""Bash""#),
        ]),
        request(&[("tools", &tools(&["Read"])), ("tool_choice", r#""none""#),])
    );
}

#[test]
fn the_rewrite_changes_nothing_when_nothing_is_forbidden() {
    let listed_tools = tools(&["Read"]);
    let listed_functions = functions(&["Read"]);
    let fields = [
        ("tools", listed_tools.as_str()),
        ("functions", listed_functions.as_str()),
        ("tool_choice", r#""Read""#),
    ];
    assert_eq!(rewritten(&fields), request(&fields));
}
