#![cfg(test)]

use std::collections::BTreeMap;

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
fn claude_code_response_prefers_result_text_and_structured_output() {
    let mut state = ClaudeCodeStreamState::default();
    let result: JsonObject = serde_json::from_str(
        r#"{"type":"result","session_id":"s1","result":"final","structured_output":{"ok":true}}"#,
    )
    .expect("valid result");
    super::stream::transform(&result, &mut state);
    assert_eq!(state.session_id.as_deref(), Some("s1"));
    assert_eq!(state.final_text(), "final");
    assert_eq!(state.structured_output_json(), r#"{"ok":true}"#);
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
