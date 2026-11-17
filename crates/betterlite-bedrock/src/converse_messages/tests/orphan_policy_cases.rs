use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

use crate::converse_messages::error::ConverseMessageError;
use crate::converse_messages::orphans::{OrphanPolicy, handle_orphaned_tool_blocks};
use crate::model_routes::ParamMap;

use super::support::{
    Warnings, assistant_turn, expected, messages, params, render, render_params, timeline,
    user_turn,
};

const ORPHAN: &str = r#"[
    {"role": "user", "content": "question"},
    {"role": "assistant", "content": "calling", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "result"}
]"#;
const EMPTY_CALLS: &str = r#"[{"role": "assistant", "content": "", "tool_calls": []}]"#;
const DUMMY_TOOLS: &str = r#"[{"type": "function", "function": {"name": "dummy_tool", "description": "This is a dummy tool call", "parameters": {"type": "object", "properties": {}}}}]"#;
const TOOL_CALLS_WITHOUT_TOOLS: &str = "Bedrock doesn't support tool calling without `tools=` param specified. Pass `tools=` param OR set `litellm.modify_params = True` // `litellm_settings::modify_params: True` to add dummy tool to the request.";
const WARNING: &str = "litellm.bedrock: request has tool blocks in message history but no `tools=` param; neutralizing orphaned tool blocks to text so Bedrock accepts the request without a toolConfig. Non-text tool-result payloads are dropped. Pass `tools=` to preserve structured tool calling.";

const LEGACY: OrphanPolicy = OrphanPolicy {
    neutralize: false,
    modify_params: false,
};
const MODIFYING: OrphanPolicy = OrphanPolicy {
    neutralize: false,
    modify_params: true,
};
const NEUTRALIZE: OrphanPolicy = OrphanPolicy {
    neutralize: true,
    modify_params: false,
};
const NEUTRALIZING_MODIFY: OrphanPolicy = OrphanPolicy {
    neutralize: true,
    modify_params: true,
};

fn handle(history: &str, optional: &str, policy: OrphanPolicy) -> (Vec<Message>, ParamMap) {
    let mut parameters = params(optional);
    let kept = handle_orphaned_tool_blocks(&messages(history), &mut parameters, policy)
        .expect("the messages handle");
    (kept, parameters)
}

fn failure(history: &str, optional: &str, policy: OrphanPolicy) -> ConverseMessageError {
    let mut parameters = params(optional);
    handle_orphaned_tool_blocks(&messages(history), &mut parameters, policy)
        .expect_err("the messages are rejected")
}

fn with_dummy_tools() -> Json {
    expected(&format!(r#"{{"tools": {DUMMY_TOOLS}}}"#))
}

fn neutralized_orphan() -> Json {
    timeline(&[
        user_turn("question"),
        assistant_turn("calling\\n[tool call call_1: lookup({})]"),
        user_turn("[tool result for call_1: result]"),
    ])
}

pub(super) fn orphan_tool_with_modify_params() {
    modify_params_adds_the_dummy_tools();
    a_tools_key_skips_the_legacy_check();
    only_tool_call_blocks_trigger_the_legacy_path();
    neutralization_wins_over_modify_params();
}

fn modify_params_adds_the_dummy_tools() {
    let (kept, parameters) = handle(ORPHAN, "{}", MODIFYING);
    assert_eq!(render(&kept), expected(ORPHAN));
    assert_eq!(render_params(&parameters), with_dummy_tools());

    let (kept, parameters) = handle(EMPTY_CALLS, "{}", MODIFYING);
    assert_eq!(render(&kept), expected(EMPTY_CALLS));
    assert_eq!(
        render_params(&parameters),
        with_dummy_tools(),
        "an empty tool_calls list still counts"
    );

    let user_calls = r#"[{"role": "user", "content": "x", "tool_calls": [{"id": "c", "type": "function", "function": {"name": "n", "arguments": ""}}]}]"#;
    let (kept, parameters) = handle(user_calls, "{}", MODIFYING);
    assert_eq!(render(&kept), expected(user_calls));
    assert_eq!(
        render_params(&parameters),
        with_dummy_tools(),
        "any role may carry the tool calls"
    );
}

fn a_tools_key_skips_the_legacy_check() {
    for tools in ["[]", "null", "{}", r#"{"unused": true}"#] {
        let optional = format!(r#"{{"tools": {tools}}}"#);
        let (kept, parameters) = handle(ORPHAN, &optional, MODIFYING);
        assert_eq!(render(&kept), expected(ORPHAN), "{tools}");
        assert_eq!(render_params(&parameters), expected(&optional), "{tools}");
    }
}

fn only_tool_call_blocks_trigger_the_legacy_path() {
    let null_calls = r#"[{"role": "assistant", "content": "x", "tool_calls": null}]"#;
    let (kept, parameters) = handle(null_calls, "{}", MODIFYING);
    assert_eq!(render(&kept), expected(null_calls));
    assert!(parameters.is_empty(), "a null tool_calls is not a block");

    for plain in [
        r#"[{"role": "tool", "tool_call_id": "c", "content": "r"}]"#,
        r#"[{"role": "function", "name": "f", "content": "r"}]"#,
    ] {
        let (kept, parameters) = handle(plain, "{}", MODIFYING);
        assert_eq!(render(&kept), expected(plain));
        assert!(parameters.is_empty(), "{plain}");
    }
}

fn neutralization_wins_over_modify_params() {
    let (kept, parameters) = handle(ORPHAN, "{}", NEUTRALIZING_MODIFY);
    assert_eq!(
        render(&kept),
        neutralized_orphan(),
        "neutralization wins over modify_params"
    );
    assert!(parameters.is_empty());
}

pub(super) fn orphan_tool_error() {
    let mut parameters = params("{}");
    let error = handle_orphaned_tool_blocks(&messages(ORPHAN), &mut parameters, LEGACY)
        .expect_err("tool calls without tools are rejected");
    assert!(matches!(error, ConverseMessageError::ToolCallsWithoutTools));
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(error.to_string(), TOOL_CALLS_WITHOUT_TOOLS);
    assert!(parameters.is_empty());

    assert!(
        matches!(
            failure(EMPTY_CALLS, "{}", LEGACY),
            ConverseMessageError::ToolCallsWithoutTools
        ),
        "an empty tool_calls list still rejects"
    );

    for tools in ["null", "[]", "{}"] {
        let optional = format!(r#"{{"tools": {tools}}}"#);
        let (kept, parameters) = handle(ORPHAN, &optional, LEGACY);
        assert_eq!(render(&kept), expected(ORPHAN), "{tools}");
        assert_eq!(render_params(&parameters), expected(&optional), "{tools}");
    }

    let plain = r#"[{"role": "tool", "content": "result"}]"#;
    let (kept, _) = handle(plain, "{}", LEGACY);
    assert_eq!(render(&kept), expected(plain));
    let (kept, _) = handle("[]", "{}", LEGACY);
    assert_eq!(render(&kept), expected("[]"));
    let (kept, _) = handle(ORPHAN, "{}", NEUTRALIZE);
    assert_eq!(render(&kept), neutralized_orphan());
}

pub(super) fn orphan_neutralization_default() {
    the_rewrite_warns();
    falsy_tools_still_rewrite();
    truthy_tools_keep_the_tool_blocks();
    the_default_path_ignores_modify_params();
    an_empty_tool_calls_list_stays();
    a_request_without_orphans_stays_quiet();
}

fn the_rewrite_warns() {
    let warnings = Warnings::default();
    let guard = warnings.install();
    let (kept, parameters) = handle(ORPHAN, "{}", NEUTRALIZE);
    assert_eq!(render(&kept), neutralized_orphan());
    assert!(parameters.is_empty());
    let captured = warnings.captured();
    assert!(captured.contains("WARN"), "{captured}");
    assert!(captured.contains(WARNING), "{captured}");
    drop(guard);
}

fn falsy_tools_still_rewrite() {
    for tools in ["[]", "null", "{}", "false", "\"\"", "0"] {
        let optional = format!(r#"{{"tools": {tools}}}"#);
        let (kept, _) = handle(ORPHAN, &optional, NEUTRALIZE);
        assert_eq!(render(&kept), neutralized_orphan(), "{tools}");
    }
}

fn truthy_tools_keep_the_tool_blocks() {
    let tools = r#"{"tools": [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}]}"#;
    let (kept, parameters) = handle(ORPHAN, tools, NEUTRALIZE);
    assert_eq!(render(&kept), expected(ORPHAN));
    assert_eq!(render_params(&parameters), expected(tools));
}

fn the_default_path_ignores_modify_params() {
    let (kept, parameters) = handle(ORPHAN, "{}", NEUTRALIZING_MODIFY);
    assert_eq!(render(&kept), neutralized_orphan());
    assert!(parameters.is_empty(), "a rewrite does not add dummy tools");
}

fn an_empty_tool_calls_list_stays() {
    let history = r#"[{"role": "assistant", "content": "x", "tool_calls": []}, {"role": "tool", "tool_call_id": "c", "content": "r"}]"#;
    let (kept, _) = handle(history, "{}", NEUTRALIZE);
    assert_eq!(
        render(&kept),
        timeline(&[
            r#"{"role": "assistant", "content": "x", "tool_calls": []}"#.to_owned(),
            user_turn("[tool result for c: r]"),
        ])
    );
}

fn a_request_without_orphans_stays_quiet() {
    let quiet = Warnings::default();
    let quiet_guard = quiet.install();
    let plain = r#"[{"role": "user", "content": "hello"}, {"role": "assistant", "content": "hi"}]"#;
    let (kept, _) = handle(plain, "{}", NEUTRALIZE);
    assert_eq!(render(&kept), expected(plain));
    let tools = r#"{"tools": [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}]}"#;
    let (kept, _) = handle(ORPHAN, tools, NEUTRALIZE);
    assert_eq!(render(&kept), expected(ORPHAN));
    let captured = quiet.captured();
    assert!(captured.is_empty(), "{captured}");
    drop(quiet_guard);
}
