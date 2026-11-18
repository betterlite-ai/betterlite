use super::support::{
    CLAUDE_3_5, CLAUDE_4_5, Call, Outcome, Settings, expected, field, remote, run,
};
use crate::converse_request::ConverseMessageSettings;

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
const THINKING_ENABLED: &str = r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}}"#;
const THINKING_DISABLED: &str = r#"{"thinking": {"type": "disabled"}}"#;
const MODIFYING: Settings = Settings {
    global_drop: false,
    messages: ConverseMessageSettings {
        modify_params: true,
        neutralize_orphans: false,
    },
};

// A turn whose own tool history carries no thinking drops the param, and the caller options lose it too.
pub(super) fn thinking_history_policy() {
    tool_history_drops_thinking();
    thinking_kept_without_modify_params();
    disabled_thinking_gate();
}

fn tool_history_drops_thinking() {
    let enabled = thinking_with_tools();
    let call = Call {
        messages: TOOL_TURN,
        settings: MODIFYING,
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (dropped, warnings) = run(&call, &remote(&[]));
    assert!(field(dropped.body(), "additionalModelRequestFields").is_none());
    assert!(field(dropped.body(), "additionalModelResponseFieldPaths").is_none());
    assert!(field(&dropped.optional_params, "thinking").is_none());
    assert!(warnings.contains(DROP_THINKING_WARNING), "{warnings}");
}

fn thinking_kept_without_modify_params() {
    let enabled = thinking_with_tools();

    // Without modify_params the history drop does not run at all.
    let call = Call {
        messages: TOOL_TURN,
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (kept, warnings) = run(&call, &remote(&[]));
    assert_thinking_kept(&kept, &warnings);

    // One assistant message with thinking keeps the param enabled for the whole turn.
    let thought_call = Call {
        messages: THOUGHT_TURN,
        settings: MODIFYING,
        ..Call::new(CLAUDE_4_5, &enabled)
    };
    let (thought, thought_warnings) = run(&thought_call, &remote(&[]));
    assert_thinking_kept(&thought, &thought_warnings);
}

fn disabled_thinking_gate() {
    // A disabled thinking block survives on a model without always-on thinking and still asks for the field paths.
    let (survived, warnings) = run(&Call::new(CLAUDE_3_5, THINKING_DISABLED), &remote(&[]));
    assert_eq!(
        field(survived.body(), "additionalModelRequestFields"),
        Some(&expected(THINKING_DISABLED))
    );
    assert_eq!(
        field(survived.body(), "additionalModelResponseFieldPaths"),
        Some(&expected(OUTPUT_PATHS))
    );
    assert!(!warnings.contains("thinking is always on"), "{warnings}");

    // On an always-on model the disabled block is dropped and leaves no response path behind.
    let (removed, removed_warnings) = run(&Call::new(CLAUDE_4_5, THINKING_DISABLED), &remote(&[]));
    assert!(field(removed.body(), "additionalModelRequestFields").is_none());
    assert!(field(removed.body(), "additionalModelResponseFieldPaths").is_none());
    assert!(
        removed_warnings.contains("thinking is always on"),
        "{removed_warnings}"
    );
    assert!(removed_warnings.contains(CLAUDE_4_5), "{removed_warnings}");
}

fn thinking_with_tools() -> String {
    format!(r#"{{"thinking": {{"type": "enabled", "budget_tokens": 1024}}, {TOOL}}}"#)
}

fn assert_thinking_kept(outcome: &Outcome, warnings: &str) {
    assert_eq!(
        field(outcome.body(), "additionalModelRequestFields"),
        Some(&expected(THINKING_ENABLED))
    );
    assert_eq!(
        field(outcome.body(), "additionalModelResponseFieldPaths"),
        Some(&expected(OUTPUT_PATHS))
    );
    assert!(!warnings.contains(DROP_THINKING_WARNING), "{warnings}");
}
