#![cfg(test)]

use super::*;

#[test]
fn normalizes_and_reports_dropped_codex_items() {
    let input = serde_json::json!([{"type":"agent_message","content":[{"text":"a"},{"encrypted_content":"b"},{"text":0,"encrypted_content":"c"},7]},{"type":"context_compaction","encrypted_content":""},7]);
    let (output, types) = normalize_codex_input_items(&input);
    assert_eq!(types, ["agent_message", "context_compaction"]);
    assert_eq!(
        output,
        serde_json::json!([{ "type":"message","role":"assistant","content":[{"type":"output_text","text":"abc"}]},7])
    );
}

#[test]
fn hoists_vendor_tools_and_keeps_existing_order() {
    let input = serde_json::json!([{ "type":"message"},{"type":"additional_tools","tools":[{"type":"vendor","name":"extra","custom":7}]},{"type":"additional_tools","tools":7}]);
    let existing = [serde_json::json!({"type":"function","name":"first"})];
    let result = hoist_additional_tools(&input, Some(&existing));
    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.hoisted.len(), 1);
    assert_eq!(result.input, serde_json::json!([{ "type":"message"}]));
    assert_eq!(
        result.debug.as_deref(),
        Some(
            "Responses API: hoisting 1 tool(s) out of 2 'additional_tools' input item(s) into the top-level tools param."
        )
    );
}

#[test]
fn malformed_tool_list_contributes_no_tools() {
    let input = serde_json::json!([{
        "type":"additional_tools",
        "tools":[{"type":"vendor"}, "malformed"]
    }]);
    let result = hoist_additional_tools(&input, None);
    assert!(result.hoisted.is_empty());
    assert!(result.tools.is_empty());
}

#[test]
fn agent_message_uses_fallback_after_empty_container() {
    let input = serde_json::json!([{
        "type":"agent_message",
        "content":[{"text":[],"encrypted_content":"fallback"}]
    }]);
    let (output, _) = normalize_codex_input_items(&input);
    assert_eq!(
        output,
        serde_json::json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"fallback"}]}])
    );
}

#[test]
fn local_shell_arguments_escape_non_ascii_like_python_json() {
    let input = serde_json::json!([{
        "type":"local_shell_call",
        "call_id":"c",
        "action":{"h\u{e9}":"\u{e9}\u{1f600}"}
    }]);
    let (output, _) = normalize_codex_input_items(&input);
    assert_eq!(
        output,
        serde_json::json!([{"type":"function_call","call_id":"c","name":"local_shell","arguments":"{\"h\\u00e9\": \"\\u00e9\\ud83d\\ude00\"}"}])
    );
}
