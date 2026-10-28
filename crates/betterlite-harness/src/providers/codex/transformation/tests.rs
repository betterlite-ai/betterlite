#![cfg(test)]

use super::{CodexHarnessConfig, CodexStreamState};
use crate::events::Event;
use crate::transformation::BaseCLIHarnessConfig as _;
use crate::types::JsonObject;

#[test]
fn empty_toml_key_is_quoted() {
    assert_eq!(super::toml_key(""), "\"\"");
}

#[test]
fn config_toml_managed_keys() {
    assert_eq!(super::toml_key("plain-key"), "plain-key");
    assert_eq!(super::toml_key("a key"), "\"a key\"");
    let config: JsonObject =
        serde_json::from_str("{\"a key\": [true, 2], \"model\": \"wrong\"}").unwrap();
    let value = config.values().next().unwrap();
    assert_eq!(super::toml_value(value, 0).unwrap(), "[true, 2]");
    super::config_overrides(&config).unwrap_err();
}

#[test]
fn stream_tool_call_precedes_result() {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"item.completed","item":{"id":"tool-1","type":"command_execution","command":"echo hi","aggregated_output":"hi\n","exit_code":0,"status":"completed"}}"#,
    )
    .unwrap();
    let events = CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert!(matches!(events.first(), Some(Event::ToolCall { .. })));
    assert!(matches!(
        events.get(1),
        Some(Event::ToolResult {
            is_error: false,
            ..
        })
    ));
}
