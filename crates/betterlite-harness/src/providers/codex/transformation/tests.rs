#![cfg(test)]

use std::collections::BTreeMap;

use super::{CodexHarnessConfig, CodexStreamState};
use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{ClaudeCodeOptions, CodexOptions, HarnessOptions, ReasoningEffort};
use crate::payloads::DEFAULT_MAX_RECURSE_DEPTH;
use crate::sandbox::local::LocalSandbox;
use crate::transformation::BaseCLIHarnessConfig as _;
use crate::types::{Harness, JsonObject};

#[test]
fn empty_toml_key_is_quoted() {
    assert_eq!(super::config::toml_key(""), "\"\"");
}

#[test]
fn config_toml_managed_keys() {
    assert_eq!(super::config::toml_key("plain-key"), "plain-key");
    assert_eq!(super::config::toml_key("a key"), "\"a key\"");
    let config: JsonObject =
        serde_json::from_str("{\"a key\": [true, 2], \"model\": \"wrong\"}").unwrap();
    let value = config.values().next().unwrap();
    assert_eq!(super::config::toml_value(value, 0).unwrap(), "[true, 2]");
    super::config::config_overrides(&config).unwrap_err();
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

#[rstest::rstest]
#[case::null_error(r#"{"type":"mcp_tool_call","error":null,"result":"ok"}"#, "ok", false)]
#[case::null_result(r#"{"type":"mcp_tool_call","result":null}"#, "", false)]
#[case::string_error(r#"{"type":"mcp_tool_call","error":"broken"}"#, "broken", true)]
fn mcp_output_matches_nulls(#[case] raw: &str, #[case] expected: &str, #[case] is_error: bool) {
    let item: JsonObject = serde_json::from_str(raw).unwrap();
    assert_eq!(
        super::stream::tool_output(&item),
        (expected.to_owned(), is_error)
    );
}

#[test]
fn missing_mcp_arguments_are_null() {
    let item: JsonObject =
        serde_json::from_str(r#"{"type":"mcp_tool_call","tool":"list"}"#).unwrap();
    let (_, _, input, _) = super::stream::tool_input(&item);
    assert_eq!(
        serde_json::to_string(&input).unwrap(),
        r#"{"arguments":null}"#
    );
}

#[test]
fn absent_thread_id_preserves_session() {
    let mut state = CodexStreamState::default();
    let start: JsonObject =
        serde_json::from_str(r#"{"type":"thread.started","thread_id":"thread-1"}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&start, &mut state);
    let empty: JsonObject = serde_json::from_str(r#"{"type":"thread.started"}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&empty, &mut state);
    assert_eq!(state.thread_id.as_deref(), Some("thread-1"));
}

#[test]
fn reasoning_enables_summaries() {
    let context = SessionContext {
        harness: Harness::Codex,
        sandbox: (),
        session_id: "session-1".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    let options = CodexOptions {
        reasoning_effort: Some(ReasoningEffort::High),
        web_search: false,
        config: JsonObject::new(),
        env: BTreeMap::new(),
    };
    let overrides = super::features::feature_overrides(&context, &options);
    assert!(
        overrides
            .iter()
            .any(|value| value == "model_supports_reasoning_summaries=true")
    );
}

#[rstest::rstest]
#[case::boolean("true", "true")]
#[case::integer("42", "42")]
#[case::decimal("1.5", "1.5")]
#[case::string(r#""quoted\"text""#, r#""quoted\"text""#)]
#[case::table(r#"{"": [true, 2]}"#, r#"{"" = [true, 2]}"#)]
fn config_values(#[case] raw: &str, #[case] expected: &str) {
    let config: JsonObject = serde_json::from_str(&format!("{{\"value\":{raw}}}")).unwrap();
    let value = &config["value"];
    assert_eq!(super::config::toml_value(value, 0).unwrap(), expected);
    super::config::toml_value(value, DEFAULT_MAX_RECURSE_DEPTH + 1).unwrap_err();
}

#[rstest::rstest]
#[case::provider("model_providers.litellm.base_url")]
#[case::model("model")]
#[case::approval("approval_policy")]
#[case::sandbox("sandbox_mode")]
#[case::mcp("mcp_servers")]
#[case::instructions("developer_instructions")]
#[case::search("web_search")]
fn rejects_managed_config(#[case] key: &str) {
    let config: JsonObject = serde_json::from_str(&format!("{{\"{key}\":true}}")).unwrap();
    super::config::config_overrides(&config).unwrap_err();
}

#[test]
fn accepts_network_config() {
    let config: JsonObject =
        serde_json::from_str(r#"{"sandbox_workspace_write.network_access":true}"#).unwrap();
    assert_eq!(
        super::config::config_overrides(&config).unwrap(),
        ["sandbox_workspace_write.network_access=true"]
    );
}

fn session_context() -> (tempfile::TempDir, SessionContext<LocalSandbox>) {
    let root = tempfile::tempdir().unwrap();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let settings = SessionSettings {
        model: Some("gpt-test".to_owned()),
        gateway: Some(GatewayTarget {
            api_base: "http://localhost:1234/".to_owned(),
            api_key: secrecy::SecretString::from("private-token"),
        }),
        ..SessionSettings::default()
    };
    let context = SessionContext {
        harness: Harness::Codex,
        sandbox,
        session_id: "session-1".to_owned(),
        settings,
        outcome: TurnOutcome::default(),
    };
    (root, context)
}

#[test]
fn setup_rejects_other_options() {
    let (_root, mut context) = session_context();
    context.settings.options = Some(HarnessOptions::ClaudeCode(ClaudeCodeOptions::default()));
    let result = CodexHarnessConfig.transform_session_setup(&context, "/private");
    assert!(matches!(
        result,
        Err(HarnessError::OptionsMismatch {
            given: "ClaudeCodeOptions",
            harness: Harness::Codex,
            expected: "CodexOptions",
        })
    ));
}

#[rstest::rstest]
#[case::first(None)]
#[case::resume(Some("thread-1"))]
fn turn_is_private(#[case] native_session_id: Option<&str>) {
    use crate::sandbox::Sandbox as _;
    use secrecy::ExposeSecret as _;

    let (_root, context) = session_context();
    let setup = CodexHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert_eq!(setup.env["CODEX_HOME"].expose_secret(), "/private");
    assert_eq!(
        setup.env["LITELLM_HARNESS_TOKEN"].expose_secret(),
        "private-token"
    );
    assert_eq!(setup.skills_dir.as_deref(), Some("skills"));
    assert_eq!(
        setup.persisted_dirs.first().unwrap().cache_path,
        "codex/sessions"
    );
    assert!(setup.files.is_empty());
    let request = CodexHarnessConfig
        .transform_turn_request(
            &context,
            crate::transformation::TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "do work",
                native_session_id,
            },
        )
        .unwrap();
    assert_eq!(request.stdin, "do work");
    assert_eq!(request.argv.last().map(String::as_str), Some("-"));
    assert!(!request.argv.iter().any(|arg| arg.contains("private-token")));
    assert!(
        request
            .argv
            .iter()
            .any(|arg| arg == "model_providers.litellm.env_key=LITELLM_HARNESS_TOKEN")
    );
    assert!(
        request
            .argv
            .iter()
            .any(|arg| arg == "model_providers.litellm.base_url=\"http://localhost:1234/v1\"")
    );
    assert_eq!(request.cwd.as_deref(), Some(context.sandbox.workdir()));
    if native_session_id.is_some() {
        assert_eq!(request.argv.get(2).map(String::as_str), Some("resume"));
        assert!(
            !request
                .argv
                .iter()
                .any(|arg| arg == "--sandbox" || arg == "-C")
        );
        assert!(
            request
                .argv
                .iter()
                .any(|arg| arg.starts_with("sandbox_mode="))
        );
    } else {
        assert!(request.argv.iter().any(|arg| arg == "--sandbox"));
        assert!(request.argv.iter().any(|arg| arg == "-C"));
    }
}

#[test]
fn structured_output_and_failures() {
    let (_root, mut context) = session_context();
    context.settings.output_schema = Some(
        serde_json::from_str(r#"{"type":"object","properties":{"answer":{"type":"string"}}}"#)
            .unwrap(),
    );
    let setup = CodexHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert!(setup.files.contains_key("output_schema.json"));
    let mut state = CodexStreamState {
        final_text: r#"{"answer":"yes"}"#.to_owned(),
        ..CodexStreamState::default()
    };
    let response = CodexHarnessConfig
        .transform_turn_response(&context, &state, 0, &[])
        .unwrap();
    assert_eq!(
        response.output_json.as_deref(),
        Some(state.final_text.as_str())
    );
    context.settings.output_schema = None;
    assert!(
        CodexHarnessConfig
            .transform_turn_response(&context, &state, 0, &[])
            .unwrap()
            .output_json
            .is_none()
    );
    CodexHarnessConfig
        .transform_turn_response(&context, &state, 1, &[])
        .unwrap_err();
    state.failed = true;
    CodexHarnessConfig
        .transform_turn_response(&context, &state, 0, &[])
        .unwrap_err();
}
