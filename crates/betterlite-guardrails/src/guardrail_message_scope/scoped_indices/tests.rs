#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    GlobalSkipFlags, GuardrailSkipFlags, ScanScope, ScopeFlags, filter_messages_by_skip_flags,
    message_role, role_out_of_guardrail_scope, scoped_message_indices,
};

fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn messages(source: &str) -> Vec<GuardrailValue> {
    let GuardrailValue::Array(items) = json(source) else {
        panic!("not an array: {source}");
    };
    items
}

fn scope(skips: [bool; 2], scan: ScanScope) -> ScopeFlags {
    let [skip_system_message, skip_tool_message] = skips;
    ScopeFlags {
        skip_system_message,
        skip_tool_message,
        scan,
    }
}

fn own(
    system: Option<bool>,
    tool: Option<bool>,
    only_tool_results: Option<bool>,
) -> GuardrailSkipFlags {
    GuardrailSkipFlags {
        skip_system_message: system,
        skip_tool_message: tool,
        scan_only_tool_results: only_tool_results,
    }
}

fn global_flags(skips: [bool; 2]) -> GlobalSkipFlags {
    let [skip_system_message, skip_tool_message] = skips;
    GlobalSkipFlags {
        skip_system_message,
        skip_tool_message,
    }
}

#[rstest]
#[case::unset_everywhere(
    own(None, None, None),
    global_flags([false, false]),
    scope([false, false], ScanScope::AllMessages)
)]
#[case::own_true_wins_over_global_false(
    own(Some(true), Some(true), None),
    global_flags([false, false]),
    scope([true, true], ScanScope::AllMessages)
)]
#[case::own_false_wins_over_global_true(
    own(Some(false), Some(false), None),
    global_flags([true, true]),
    scope([false, false], ScanScope::AllMessages)
)]
#[case::unset_falls_back_to_global(
    own(None, None, None),
    global_flags([true, true]),
    scope([true, true], ScanScope::AllMessages)
)]
#[case::system_and_tool_resolve_apart(
    own(Some(false), None, None),
    global_flags([true, true]),
    scope([false, true], ScanScope::AllMessages)
)]
#[case::tool_own_flag_only(
    own(None, Some(false), None),
    global_flags([true, true]),
    scope([true, false], ScanScope::AllMessages)
)]
#[case::scan_only_tool_results(
    own(None, None, Some(true)),
    global_flags([false, false]),
    scope([false, false], ScanScope::ToolResultsOnly)
)]
#[case::scan_only_tool_results_false(
    own(None, None, Some(false)),
    global_flags([false, false]),
    scope([false, false], ScanScope::AllMessages)
)]
fn flags_resolve_guardrail_first_then_global(
    #[case] guardrail: GuardrailSkipFlags,
    #[case] global: GlobalSkipFlags,
    #[case] expected: ScopeFlags,
) {
    assert_eq!(ScopeFlags::resolve(&guardrail, &global), expected);
}

#[rstest]
#[case::lowercase(r#"{"role":"user"}"#, "user")]
#[case::uppercase_is_folded(r#"{"role":"SYSTEM"}"#, "system")]
#[case::mixed_case(r#"{"role":"Tool"}"#, "tool")]
#[case::missing(r#"{"content":"x"}"#, "")]
#[case::null(r#"{"role":null}"#, "")]
#[case::not_text(r#"{"role":3}"#, "")]
#[case::empty(r#"{"role":""}"#, "")]
#[case::message_is_not_an_object(r#""system""#, "")]
fn the_role_is_lowercase_text_or_empty(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(message_role(&json(source)), expected);
}

#[rstest]
#[case::nothing_skipped_system(scope([false, false], ScanScope::AllMessages), "system", false)]
#[case::nothing_skipped_tool(scope([false, false], ScanScope::AllMessages), "tool", false)]
#[case::system_skipped(scope([true, false], ScanScope::AllMessages), "system", true)]
#[case::system_skip_keeps_user(scope([true, false], ScanScope::AllMessages), "user", false)]
#[case::system_skip_keeps_tool(scope([true, false], ScanScope::AllMessages), "tool", false)]
#[case::tool_skipped(scope([false, true], ScanScope::AllMessages), "tool", true)]
#[case::tool_skip_keeps_function(scope([false, true], ScanScope::AllMessages), "function", false)]
#[case::tool_skip_keeps_system(scope([false, true], ScanScope::AllMessages), "system", false)]
#[case::scan_only_tool_drops_user(scope([false, false], ScanScope::ToolResultsOnly), "user", true)]
#[case::scan_only_tool_drops_empty_role(scope([false, false], ScanScope::ToolResultsOnly), "", true)]
#[case::scan_only_tool_drops_assistant(
    scope([false, false], ScanScope::ToolResultsOnly),
    "assistant",
    true
)]
#[case::scan_only_tool_keeps_tool(scope([false, false], ScanScope::ToolResultsOnly), "tool", false)]
#[case::scan_only_tool_keeps_function(
    scope([false, false], ScanScope::ToolResultsOnly),
    "function",
    false
)]
#[case::scan_only_tool_with_tool_skip_drops_tool(
    scope([false, true], ScanScope::ToolResultsOnly),
    "tool",
    true
)]
#[case::scan_only_tool_with_tool_skip_keeps_function(
    scope([false, true], ScanScope::ToolResultsOnly),
    "function",
    false
)]
fn a_role_leaves_scope_by_the_flags(
    #[case] flags: ScopeFlags,
    #[case] role: &str,
    #[case] expected: bool,
) {
    assert_eq!(role_out_of_guardrail_scope(role, &flags), expected);
}

#[rstest]
#[case::all_messages(scope([false, false], ScanScope::AllMessages), vec![0, 1, 2, 3, 4])]
#[case::skip_system(scope([true, false], ScanScope::AllMessages), vec![1, 2, 3, 4])]
#[case::skip_tool(scope([false, true], ScanScope::AllMessages), vec![0, 1, 2, 4])]
#[case::skip_both(scope([true, true], ScanScope::AllMessages), vec![1, 2, 4])]
#[case::tool_results_only(scope([false, false], ScanScope::ToolResultsOnly), vec![3, 4])]
#[case::tool_results_only_and_skip_tool(scope([false, true], ScanScope::ToolResultsOnly), vec![4])]
fn scoped_indices_point_into_the_full_conversation(
    #[case] flags: ScopeFlags,
    #[case] expected: Vec<usize>,
) {
    let conversation = messages(
        r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"tool","content":"t"},{"role":"function","content":"f"}]"#,
    );
    assert_eq!(scoped_message_indices(&conversation, &flags), expected);
}

#[test]
fn messages_that_are_not_objects_have_no_role() {
    let conversation = messages(r#"["system", {"role":"system"}, 3, null]"#);
    let flags = scope([true, false], ScanScope::AllMessages);
    assert_eq!(scoped_message_indices(&conversation, &flags), vec![0, 2, 3]);
}

#[test]
fn an_empty_conversation_has_no_scoped_indices() {
    let flags = scope([true, true], ScanScope::ToolResultsOnly);
    assert!(scoped_message_indices(&[], &flags).is_empty());
}

#[rstest]
#[case::nothing_skipped(scope([false, false], ScanScope::AllMessages), 4, false)]
#[case::skip_system(scope([true, false], ScanScope::AllMessages), 3, true)]
#[case::skip_tool(scope([false, true], ScanScope::AllMessages), 3, true)]
#[case::skip_both(scope([true, true], ScanScope::AllMessages), 2, true)]
#[case::scan_only_tool_results_drops_nothing_here(
    scope([false, false], ScanScope::ToolResultsOnly),
    4,
    false
)]
fn skip_flags_filter_the_message_list(
    #[case] flags: ScopeFlags,
    #[case] kept: usize,
    #[case] changed: bool,
) {
    let conversation = messages(
        r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"tool","content":"t"},{"role":"function","content":"f"}]"#,
    );
    let (filtered, was_changed) = filter_messages_by_skip_flags(&conversation, &flags);
    assert_eq!((filtered.len(), was_changed), (kept, changed));
}

#[test]
fn filtering_keeps_the_original_order_and_roles() {
    let conversation = messages(
        r#"[{"role":"system"},{"role":"user"},{"role":"tool"},{"role":"function"},{"role":"assistant"}]"#,
    );
    let (filtered, _) =
        filter_messages_by_skip_flags(&conversation, &scope([true, true], ScanScope::AllMessages));
    let roles: Vec<String> = filtered.iter().map(message_role).collect();
    assert_eq!(roles, ["user", "function", "assistant"]);
}

#[test]
fn an_uppercase_role_is_skipped_too() {
    let conversation = messages(r#"[{"role":"SYSTEM"},{"role":"Tool"}]"#);
    let (filtered, changed) =
        filter_messages_by_skip_flags(&conversation, &scope([true, true], ScanScope::AllMessages));
    assert!(filtered.is_empty());
    assert!(changed);
}
