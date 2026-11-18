use super::support::{CLAUDE_3_5, CLAUDE_4_5, Call, Settings, expected, field, remote, run};

const DROP_THINKING_WARNING: &str = "Dropping 'thinking' param because the last assistant message with tool_calls has no thinking_blocks. The model won't use extended thinking for this turn.";

const TOOL: &str = r#""tools": [{"type": "function", "function": {
    "name": "lookup",
    "description": "finds things",
    "parameters": {"type": "object", "properties": {}}
}}]"#;

const TOOL_TURN: &str = r#"[
    {"role": "user", "content": "go"},
    {"role": "assistant", "content": "", "tool_calls": [
        {"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}
    ]},
    {"role": "tool", "tool_call_id": "call_1", "content": "{}"}
]"#;

// The prior assistant's own thinking keeps the param despite the later tool call.
const THOUGHT_TURN: &str = r#"[
    {"role": "user", "content": "go"},
    {"role": "assistant", "content": "plan", "thinking_blocks": [
        {"type": "thinking", "thinking": "t", "signature": "sig"}
    ]},
    {"role": "assistant", "content": "", "tool_calls": [
        {"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}
    ]},
    {"role": "tool", "tool_call_id": "call_1", "content": "{}"}
]"#;

const OUTPUT_PATHS: &str = r#"["/usage/output_tokens_details"]"#;

// A turn whose own tool history carries no thinking drops the param, and the caller options lose it too.
pub(super) fn thinking_history_policy() {
    let enabled =
        format!(r#"{{"thinking": {{"type": "enabled", "budget_tokens": 1024}}, {TOOL}}}"#);
    let call = Call {
        messages: TOOL_TURN,
        settings: Settings {
            modify_params: true,
            ..Settings::default()
        },
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (dropped, warnings) = run(&call, &remote(&[]));
    assert!(field(dropped.body(), "additionalModelRequestFields").is_none());
    assert!(field(dropped.body(), "additionalModelResponseFieldPaths").is_none());
    assert!(field(&dropped.optional_params, "thinking").is_none());
    assert!(warnings.contains(DROP_THINKING_WARNING), "{warnings}");

    // One assistant message with thinking keeps the param enabled for the whole turn.
    let call = Call {
        messages: THOUGHT_TURN,
        settings: Settings {
            modify_params: true,
            ..Settings::default()
        },
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (thought, warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(thought.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}}"#
        ))
    );
    assert_eq!(
        field(thought.body(), "additionalModelResponseFieldPaths"),
        Some(&expected(OUTPUT_PATHS))
    );
    assert!(!warnings.contains(DROP_THINKING_WARNING), "{warnings}");

    // Without modify_params the history drop does not run at all.
    let call = Call {
        messages: TOOL_TURN,
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (kept, warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(kept.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}}"#
        ))
    );
    assert_eq!(
        field(kept.body(), "additionalModelResponseFieldPaths"),
        Some(&expected(OUTPUT_PATHS))
    );
    assert!(!warnings.contains(DROP_THINKING_WARNING), "{warnings}");

    // A disabled thinking block survives on a model without always-on thinking and still asks for the field paths.
    let disabled = r#"{"thinking": {"type": "disabled"}}"#;
    let (survived, warnings) = run(&Call::new(CLAUDE_3_5, disabled), &remote(&[]));
    assert_eq!(
        field(survived.body(), "additionalModelRequestFields"),
        Some(&expected(disabled))
    );
    assert_eq!(
        field(survived.body(), "additionalModelResponseFieldPaths"),
        Some(&expected(OUTPUT_PATHS))
    );
    assert!(!warnings.contains("thinking is always on"), "{warnings}");

    // On an always-on model the disabled block is dropped and leaves no response path behind.
    let (removed, warnings) = run(&Call::new(CLAUDE_4_5, disabled), &remote(&[]));
    assert!(field(removed.body(), "additionalModelRequestFields").is_none());
    assert!(field(removed.body(), "additionalModelResponseFieldPaths").is_none());
    assert!(warnings.contains("thinking is always on"), "{warnings}");
    assert!(warnings.contains(CLAUDE_4_5), "{warnings}");
}
