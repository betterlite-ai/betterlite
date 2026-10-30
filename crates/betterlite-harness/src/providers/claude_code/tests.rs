#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use secrecy::ExposeSecret as _;

use super::ClaudeCodeHarnessConfig;
use super::config::NATIVE_TO_NORMALIZED;
use super::stream::ClaudeCodeStreamState;
use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::events::Event;
use crate::options::ClaudeCodeOptions;
use crate::sandbox::local::LocalSandbox;
use crate::transformation::{BaseCLIHarnessConfig as _, BaseHarnessConfig as _, TurnInput};
use crate::types::{Harness, JsonObject, PermissionMode};

fn session_context() -> (tempfile::TempDir, SessionContext<LocalSandbox>) {
    let root = tempfile::tempdir().expect("tempdir");
    let sandbox = LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf())
        .expect("sandbox");
    let settings = SessionSettings {
        model: None,
        gateway: Some(GatewayTarget {
            api_base: "http://localhost:1234/".to_owned(),
            api_key: secrecy::SecretString::from("private-token"),
        }),
        api_key: None,
        api_base: None,
        instructions: None,
        skills: Vec::new(),
        disable_tools: Vec::new(),
        permissions: PermissionMode::Full,
        on_approval: None,
        output_schema: None,
        max_turns: None,
        timeout: None,
        metadata: JsonObject::new(),
        options: None,
    };
    let context = SessionContext {
        harness: Harness::ClaudeCode,
        sandbox,
        session_id: "session-1".to_owned(),
        settings,
        outcome: TurnOutcome::default(),
    };
    (root, context)
}

#[test]
fn claude_code_validation_rejects_managed_env() {
    let mut options = ClaudeCodeOptions::default();
    options
        .env
        .insert("ANTHROPIC_API_KEY".to_owned(), "bad".to_owned());
    let error =
        super::config::validate_options(&options).expect_err("managed env must be rejected");
    assert!(error.to_string().contains("ClaudeCodeOptions.env"));
}

#[test]
fn claude_code_validation_rejects_managed_config() {
    let mut options = ClaudeCodeOptions::default();
    options.config.insert(
        "model".to_owned(),
        serde_json::Value::String("bad".to_owned()),
    );
    let error =
        super::config::validate_options(&options).expect_err("managed config must be rejected");
    assert!(error.to_string().contains("model"));
}

#[test]
fn claude_code_validation_lists_all_managed_config_conflicts() {
    let mut options = ClaudeCodeOptions::default();
    options
        .config
        .insert("model".to_owned(), serde_json::Value::Null);
    options
        .config
        .insert("permissions".to_owned(), serde_json::Value::Null);
    let error =
        super::config::validate_options(&options).expect_err("managed config must be rejected");
    let message = error.to_string();
    assert!(message.contains("ClaudeCodeOptions"));
    assert!(message.contains("model"));
    assert!(message.contains("permissions"));
}

#[test]
fn claude_code_argv_permission_mapping_is_explicit() {
    assert_eq!(
        super::config::permission_mode(PermissionMode::Edit),
        "acceptEdits"
    );
}

#[test]
fn claude_code_parsing_emits_text_reasoning_and_tools() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"thinking","thinking":"why"},{"type":"tool_use","id":"t1","name":"Read","input":{"file":"a"}}]}}"#).expect("valid event");
    let events = super::stream::transform(&line, &mut state);
    assert!(matches!(events.first(), Some(Event::Text { delta }) if delta == "hi"));
    assert!(matches!(events.get(1), Some(Event::Reasoning { delta }) if delta == "why"));
    assert!(matches!(events.get(2), Some(Event::ToolCall { name, .. }) if name == "read"));
}

#[test]
fn claude_code_parsing_suppresses_parent_traffic() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"assistant","parent_tool_use_id":"p","message":{"content":[{"type":"text","text":"hidden"}]}}"#).expect("valid event");
    assert!(super::stream::transform(&line, &mut state).is_empty());
}

#[test]
fn claude_code_parsing_keeps_empty_parent_and_mixed_content() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"assistant","parent_tool_use_id":"","message":{"content":["ignored",{"type":"text","text":"kept"},{"type":"tool_use","id":"t","name":"Read","input":null}]}}"#).expect("valid event");
    let events = super::stream::transform(&line, &mut state);
    assert!(matches!(events.first(), Some(Event::Text { delta }) if delta == "kept"));
    assert!(matches!(events.get(1), Some(Event::ToolCall { input, .. }) if input.is_empty()));
}

#[test]
fn claude_code_parsing_preserves_json_tool_results() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":{"ok":true}},{"type":"tool_result","tool_use_id":"u","content":[{"type":"image","source":{"data":"x"}},"tail"]}]}}"#).expect("valid event");
    let events = super::stream::transform(&line, &mut state);
    assert!(
        matches!(events.first(), Some(Event::ToolResult { output, .. }) if output == r#"{"ok": true}"#)
    );
    assert!(
        matches!(events.get(1), Some(Event::ToolResult { output, .. }) if output.contains("image") && output.ends_with("\ntail"))
    );
}

#[test]
fn claude_code_response_ignores_non_string_result_and_empty_session() {
    let mut state = ClaudeCodeStreamState {
        session_id: Some("prior".to_owned()),
        text_parts: Vec::new(),
        result_seen: false,
        result_text: None,
        is_error: false,
        errors: Vec::new(),
        structured_output: None,
    };
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"result","session_id":"","result":{"not":"text"},"errors":["actual cause"]}"#,
    )
    .expect("valid result");
    super::stream::transform(&line, &mut state);
    assert_eq!(state.session_id.as_deref(), Some("prior"));
    assert!(
        super::stream::turn_error(&state, 1, &[])
            .is_some_and(|message| message.contains("actual cause"))
    );
}

#[test]
fn claude_code_response_keeps_missing_structured_output_absent() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject =
        serde_json::from_str(r#"{"type":"result","result":"plain text"}"#).expect("valid result");
    super::stream::transform(&line, &mut state);
    assert_eq!(state.structured_output_json(), None);
}

fn replay_fuzz_seed(name: &str) {
    let path = format!(
        "{}/fuzz/corpus/claude_code/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = fs::read_to_string(path).expect("corpus seed");
    let line: JsonObject = serde_json::from_str(&source).expect("valid corpus seed");
    let mut state = ClaudeCodeStreamState::default();
    drop(super::stream::transform(&line, &mut state));
}

#[test]
fn claude_code_fuzz_corpus_replays_mixed_content() {
    replay_fuzz_seed("mixed-content");
}

#[test]
fn claude_code_fuzz_corpus_replays_tool_output() {
    replay_fuzz_seed("tool-output");
}

#[test]
fn claude_code_response_prefers_result_text_and_structured_output() {
    let mut state = ClaudeCodeStreamState::default();
    let result: JsonObject = serde_json::from_str(
        r#"{"type":"result","session_id":"s1","result":"final","structured_output":{"ok":true}}"#,
    )
    .expect("valid result");
    super::stream::transform(&result, &mut state);
    assert_eq!(state.session_id.as_deref(), Some("s1"));
    assert_eq!(state.final_text(), "final");
    assert_eq!(
        state.structured_output_json().as_deref(),
        Some(r#"{"ok": true}"#)
    );
}

#[test]
fn claude_code_response_rejects_missing_result() {
    let state = ClaudeCodeStreamState::default();
    assert!(super::stream::turn_error(&state, 0, &[]).is_some());
}

#[test]
fn claude_code_config_exposes_expected_constants() {
    assert_eq!(ClaudeCodeHarnessConfig::HARNESS, Harness::ClaudeCode);
    assert_eq!(NATIVE_TO_NORMALIZED[0], ("Read", "read"));
}

#[test]
fn claude_code_parsing_emits_compaction_event() {
    let mut state = ClaudeCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"pre_tokens":100}}"#,
    )
    .expect("valid event");
    let events = super::stream::transform(&line, &mut state);
    assert!(matches!(
        events.first(),
        Some(Event::Compaction {
            tokens_before: Some(100),
            tokens_after: None
        })
    ));
}

#[test]
fn claude_code_argv_disallowed_tools_use_native_names() {
    let (_root, mut context) = session_context();
    context.settings.disable_tools = vec!["edit".to_owned()];
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .expect("session setup");
    let request = ClaudeCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "do work",
                native_session_id: None,
            },
        )
        .expect("turn request");
    let index = request
        .argv
        .iter()
        .position(|arg| arg == "--disallowedTools")
        .expect("--disallowedTools present");
    assert_eq!(request.argv[index.wrapping_add(1)], "Edit,MultiEdit");
}

#[test]
fn claude_code_argv_model_sets_background_env() {
    let (_root, mut context) = session_context();
    context.settings.model = Some("claude-background".to_owned());
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .expect("session setup");
    assert_eq!(
        setup.env["ANTHROPIC_MODEL"].expose_secret(),
        "claude-background"
    );
    assert_eq!(
        setup.env["ANTHROPIC_SMALL_FAST_MODEL"].expose_secret(),
        "claude-background"
    );
    let request = ClaudeCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "do work",
                native_session_id: None,
            },
        )
        .expect("turn request");
    let index = request
        .argv
        .iter()
        .position(|arg| arg == "--model")
        .expect("--model present");
    assert_eq!(request.argv[index.wrapping_add(1)], "claude-background");
}
