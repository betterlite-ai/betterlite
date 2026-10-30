#![cfg(test)]

use super::stream::ClaudeCodeStreamState;
use super::{ClaudeCodeHarnessConfig, NATIVE_TO_NORMALIZED};
use crate::events::Event;
use crate::options::ClaudeCodeOptions;
use crate::transformation::BaseHarnessConfig as _;
use crate::types::{Harness, JsonObject};

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
        super::config::permission_mode(crate::types::PermissionMode::Edit),
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
