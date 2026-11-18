use betterlite_prompts::message_content::Message;

use crate::converse_messages::orphans::neutralize_orphaned_blocks;

use super::support::{assistant_turn, expected, messages, params, render, timeline, user_turn};

const GUARDRAIL: &str = r#"{"guardrailConfig": null}"#;

fn neutralize(history: &str) -> Vec<Message> {
    neutralize_orphaned_blocks(&messages(history), &params("{}"))
}

fn guarded(history: &str) -> Vec<Message> {
    neutralize_orphaned_blocks(&messages(history), &params(GUARDRAIL))
}

pub(super) fn orphan_text_projection() {
    assistant_base_text_and_calls();
    assistant_call_defaults();
    result_labels();
    result_rendering();
    guardrail_wrapping();
    untouched_messages_keep_fields();
}

fn assistant_base_text_and_calls() {
    let check = r#"[{"role": "assistant", "content": "Let me check.", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":1}"}}]}]"#;
    assert_eq!(
        render(&neutralize(check)),
        expected(
            r#"[{"role": "assistant", "content": "Let me check.\n[tool call call_1: lookup({\"q\":1})]"}]"#
        )
    );

    let flattened = r#"[{"role": "assistant", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}], "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}]"#;
    assert_eq!(
        render(&neutralize(flattened)),
        timeline(&[assistant_turn("ab\\n[tool call call_1: lookup({})]")])
    );
}

fn assistant_call_defaults() {
    let whitespace = r#"[{"role": "assistant", "content": "  ", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": ""}}]}]"#;
    assert_eq!(
        render(&neutralize(whitespace)),
        timeline(&[assistant_turn("  \\n[tool call call_1: lookup()]")])
    );

    let no_id = r#"[{"role": "assistant", "tool_calls": [{"type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}]"#;
    assert_eq!(
        render(&neutralize(no_id)),
        timeline(&[assistant_turn("[tool call: lookup({})]")])
    );

    let no_function = r#"[{"role": "assistant", "content": "", "tool_calls": [{"id": "call_2"}]}]"#;
    assert_eq!(
        render(&neutralize(no_function)),
        timeline(&[assistant_turn("[tool call call_2: unknown_tool()]")])
    );

    let two_calls = r#"[{"role": "assistant", "content": "base", "tool_calls": [{"id": "a", "type": "function", "function": {"name": "one", "arguments": "1"}}, {"id": "b", "type": "function", "function": {"name": "two", "arguments": "2"}}]}]"#;
    assert_eq!(
        render(&neutralize(two_calls)),
        timeline(&[assistant_turn(
            "base\\n[tool call a: one(1)]\\n[tool call b: two(2)]"
        )])
    );
}

fn result_labels() {
    let identified = r#"[{"role": "tool", "tool_call_id": "call_9", "content": "sunny"}]"#;
    assert_eq!(
        render(&neutralize(identified)),
        timeline(&[user_turn("[tool result for call_9: sunny]")])
    );

    let named = r#"[{"role": "tool", "tool_call_id": "", "name": "weather", "content": "sunny"}]"#;
    assert_eq!(
        render(&neutralize(named)),
        timeline(&[user_turn("[tool result for weather: sunny]")])
    );

    let anonymous = r#"[{"role": "function", "content": "sunny"}]"#;
    assert_eq!(
        render(&neutralize(anonymous)),
        timeline(&[user_turn("[tool result for unknown: sunny]")])
    );
}

fn result_rendering() {
    let stripped = r#"[{"role": "tool", "tool_call_id": "c", "content": [{"type": "image", "source": {"bytes": "AA=="}}, {"type": "text", "text": "  hi  "}]}]"#;
    assert_eq!(
        render(&neutralize(stripped)),
        timeline(&[user_turn("[tool result for c: hi]")])
    );

    let payload = r#"[{"role": "tool", "tool_call_id": "c", "content": [{"type": "image", "source": {"bytes": "AA=="}}]}]"#;
    assert_eq!(
        render(&neutralize(payload)),
        timeline(&[user_turn(
            "[tool result for c: <non-text tool result omitted>]"
        )])
    );

    let empty = r#"[{"role": "tool", "tool_call_id": "c"}]"#;
    assert_eq!(
        render(&neutralize(empty)),
        timeline(&[user_turn(
            "[tool result for c: <non-text tool result omitted>]"
        )])
    );

    let separators = r#"[{"role": "tool", "tool_call_id": "c", "content": "\u001c"}]"#;
    assert_eq!(
        render(&neutralize(separators)),
        timeline(&[user_turn(
            "[tool result for c: <non-text tool result omitted>]"
        )])
    );
    let padded = r#"[{"role": "tool", "tool_call_id": "c", "content": "\u001chi\u001f"}]"#;
    assert_eq!(
        render(&neutralize(padded)),
        timeline(&[user_turn("[tool result for c: hi]")])
    );
}

fn guardrail_wrapping() {
    let history = r#"[{"role": "tool", "tool_call_id": "call_1", "content": "result"}, {"role": "assistant", "content": "done", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}]"#;
    assert_eq!(
        render(&guarded(history)),
        timeline(&[
            r#"{"role": "user", "content": [{"type": "guarded_text", "text": "[tool result for call_1: result]"}]}"#.to_owned(),
            r#"{"role": "assistant", "content": "done\n[tool call call_1: lookup({})]"}"#.to_owned(),
        ])
    );
    let function = r#"[{"role": "function", "name": "weather", "content": "sunny"}]"#;
    assert_eq!(
        render(&guarded(function)),
        timeline(&[
            r#"{"role": "user", "content": [{"type": "guarded_text", "text": "[tool result for weather: sunny]"}]}"#.to_owned(),
        ])
    );
}

fn untouched_messages_keep_fields() {
    let history = r#"[{"role": "user", "name": "alice", "content": "hello", "extra": {"keep": true}}, {"role": "developer", "content": "policy"}, {"role": "tool", "tool_call_id": "c", "content": "r"}, {"role": "assistant", "content": "older", "tool_calls": null}]"#;
    assert_eq!(
        render(&neutralize(history)),
        timeline(&[
            r#"{"role": "user", "name": "alice", "content": "hello", "extra": {"keep": true}}"#
                .to_owned(),
            r#"{"role": "developer", "content": "policy"}"#.to_owned(),
            user_turn("[tool result for c: r]"),
            r#"{"role": "assistant", "content": "older", "tool_calls": null}"#.to_owned(),
        ])
    );
    let assistant_named = r#"[{"role": "assistant", "name": "bot", "content": "x", "tool_calls": [{"id": "c", "type": "function", "function": {"name": "n", "arguments": "{}"}}], "extra": 1}]"#;
    assert_eq!(
        render(&neutralize(assistant_named)),
        timeline(&[assistant_turn("x\\n[tool call c: n({})]")])
    );
    let tool_named = r#"[{"role": "tool", "name": "weather", "tool_call_id": "c", "content": "r", "extra": {"dropped": true}}]"#;
    assert_eq!(
        render(&neutralize(tool_named)),
        timeline(&[user_turn("[tool result for c: r]")])
    );
}
