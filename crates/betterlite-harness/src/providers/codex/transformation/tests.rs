#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::stream::Item;
use super::{CodexHarnessConfig, CodexStreamState};
use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{ClaudeCodeOptions, CodexOptions, HarnessOptions, ReasoningEffort};
use crate::payloads::DEFAULT_MAX_RECURSE_DEPTH;
use crate::sandbox::local::LocalSandbox;
use crate::transformation::{BaseCLIHarnessConfig as _, BaseHarnessConfig as _};
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};

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
    assert!(matches!(
        super::config::config_overrides(&config).unwrap_err(),
        HarnessError::OptionsMismatch {
            given: "CodexOptions.config",
            harness: Harness::Codex,
            expected: "a key Codex does not manage itself",
        }
    ));
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
#[case::empty_string_error_is_not_error(
    r#"{"type":"mcp_tool_call","error":"","result":"ok"}"#,
    "ok",
    false
)]
#[case::empty_object_error_is_not_error(
    r#"{"type":"mcp_tool_call","error":{},"result":"ok"}"#,
    "ok",
    false
)]
#[case::false_error_is_not_error(
    r#"{"type":"mcp_tool_call","error":false,"result":"ok"}"#,
    "ok",
    false
)]
#[case::zero_error_is_not_error(r#"{"type":"mcp_tool_call","error":0,"result":"ok"}"#, "ok", false)]
#[case::object_error_without_message(r#"{"type":"mcp_tool_call","error":{}}"#, "", false)]
#[case::truthy_object_error_without_message(
    r#"{"type":"mcp_tool_call","error":{"code":1}}"#,
    "None",
    true
)]
fn mcp_output_matches_nulls(#[case] raw: &str, #[case] expected: &str, #[case] is_error: bool) {
    let item: Item = serde_json::from_str(raw).unwrap();
    let projection = super::stream::tool_projection(&item);
    assert_eq!(
        (projection.output, projection.is_error),
        (expected.to_owned(), is_error)
    );
}

#[test]
fn mcp_object_result_is_python_json_dumps() {
    let item: Item =
        serde_json::from_str(r#"{"type":"mcp_tool_call","result":{"ok":true,"n":1}}"#).unwrap();
    let projection = super::stream::tool_projection(&item);
    assert_eq!(projection.output, r#"{"ok": true, "n": 1}"#);
    assert!(!projection.is_error);
}

#[test]
fn missing_mcp_arguments_are_null() {
    let item: Item = serde_json::from_str(r#"{"type":"mcp_tool_call","tool":"list"}"#).unwrap();
    let projection = super::stream::tool_projection(&item);
    assert_eq!(
        serde_json::to_string(&projection.input).unwrap(),
        r#"{"arguments":null}"#
    );
}

#[rstest::rstest]
#[case::null(r#"{"type":"file_change","id":"f1","changes":null}"#)]
#[case::missing(r#"{"type":"file_change","id":"f1"}"#)]
fn missing_or_null_changes_are_empty_array(#[case] raw: &str) {
    let item: Item = serde_json::from_str(raw).unwrap();
    let projection = super::stream::tool_projection(&item);
    assert_eq!(
        serde_json::to_string(&projection.input["changes"]).unwrap(),
        "[]"
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
fn empty_error_message_falls_back() {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"error","message":""}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert_eq!(state.error.as_deref(), Some("codex reported an error"));
}

#[rstest::rstest]
#[case::empty_text(r#"{"type":"turn.failed","error":""}"#)]
#[case::empty_object_message(r#"{"type":"turn.failed","error":{"message":""}}"#)]
#[case::missing_message(r#"{"type":"turn.failed","error":{}}"#)]
fn empty_turn_failed_message_falls_back(#[case] raw: &str) {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(raw).unwrap();
    CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert!(state.failed);
    assert_eq!(state.error.as_deref(), Some("codex turn failed"));
}

#[rstest::rstest]
#[case::numeric_error(r#"{"type":"turn.failed","error":42}"#, "42")]
#[case::numeric_message(r#"{"type":"turn.failed","error":{"message":42}}"#, "42")]
fn turn_failed_accepts_non_string_error_shapes(#[case] raw: &str, #[case] expected: &str) {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(raw).unwrap();
    CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert!(state.failed);
    assert_eq!(state.error.as_deref(), Some(expected));
}

#[test]
fn unparsable_turn_failed_still_marks_the_turn_failed() {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(r#"{"type":"turn.failed","error":[1,2]}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert!(state.failed);
}

#[test]
fn null_item_id_still_reports_tool_events() {
    let mut state = CodexStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"item.completed","item":{"id":null,"type":"command_execution","command":"echo hi","aggregated_output":"hi\n","exit_code":0,"status":"completed"}}"#,
    )
    .unwrap();
    let events = CodexHarnessConfig.transform_stream_line(&line, &mut state);
    assert!(matches!(events.first(), Some(Event::ToolCall { id, .. }) if id.is_empty()));
    assert!(matches!(
        events.get(1),
        Some(Event::ToolResult { id, is_error: false, .. }) if id.is_empty()
    ));
}

#[test]
fn turn_failed_keeps_prior_error_when_message_is_empty() {
    let mut state = CodexStreamState::default();
    let error_line: JsonObject =
        serde_json::from_str(r#"{"type":"error","message":"earlier problem"}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&error_line, &mut state);
    let failed_line: JsonObject =
        serde_json::from_str(r#"{"type":"turn.failed","error":{}}"#).unwrap();
    CodexHarnessConfig.transform_stream_line(&failed_line, &mut state);
    assert_eq!(state.error.as_deref(), Some("earlier problem"));
}

#[rstest::rstest]
#[case::low(ReasoningEffort::Low, "low")]
#[case::medium(ReasoningEffort::Medium, "medium")]
#[case::high(ReasoningEffort::High, "high")]
#[case::xhigh(ReasoningEffort::Xhigh, "xhigh")]
fn reasoning_effort_sets_model_reasoning(#[case] effort: ReasoningEffort, #[case] value: &str) {
    let context = SessionContext {
        harness: Harness::Codex,
        sandbox: (),
        session_id: "session-1".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    let options = CodexOptions {
        reasoning_effort: Some(effort),
        web_search: false,
        config: JsonObject::new(),
        env: BTreeMap::new(),
    };
    let overrides = super::features::feature_overrides(&context, &options);
    assert_eq!(
        overrides,
        [
            "web_search=disabled".to_owned(),
            format!("model_reasoning_effort={value}"),
            "model_reasoning_summary=auto".to_owned(),
            "model_supports_reasoning_summaries=true".to_owned(),
        ]
    );
}

#[rstest::rstest]
#[case::live(true, "web_search=live")]
#[case::disabled(false, "web_search=disabled")]
fn web_search_flag_is_translated(#[case] web_search: bool, #[case] expected: &str) {
    let context = SessionContext {
        harness: Harness::Codex,
        sandbox: (),
        session_id: "session-1".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    let options = CodexOptions {
        reasoning_effort: None,
        web_search,
        config: JsonObject::new(),
        env: BTreeMap::new(),
    };
    assert_eq!(
        super::features::feature_overrides(&context, &options),
        [expected.to_owned()]
    );
}

#[test]
fn instructions_become_developer_instructions() {
    let mut context = SessionContext {
        harness: Harness::Codex,
        sandbox: (),
        session_id: "session-1".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    context.settings.instructions = Some("be terse".to_owned());
    let options = CodexOptions::default();
    assert!(
        super::features::feature_overrides(&context, &options)
            .iter()
            .any(|value| value == r#"developer_instructions="be terse""#)
    );
    context.settings.instructions = Some(String::new());
    assert!(
        !super::features::feature_overrides(&context, &options)
            .iter()
            .any(|value| value.starts_with("developer_instructions"))
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
    assert!(matches!(
        super::config::toml_value(value, DEFAULT_MAX_RECURSE_DEPTH + 1).unwrap_err(),
        HarnessError::OptionsMismatch {
            given: "CodexOptions.config value",
            harness: Harness::Codex,
            expected: "a value within the Codex config nesting limit",
        }
    ));
}

#[test]
fn null_config_value_is_rejected() {
    assert!(matches!(
        super::config::toml_value(&serde_json::Value::Null, 0).unwrap_err(),
        HarnessError::OptionsMismatch {
            given: "CodexOptions.config value",
            harness: Harness::Codex,
            expected: "a bool, number, string, array or object",
        }
    ));
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
    assert!(matches!(
        super::config::config_overrides(&config).unwrap_err(),
        HarnessError::OptionsMismatch {
            given: "CodexOptions.config",
            harness: Harness::Codex,
            expected: "a key Codex does not manage itself",
        }
    ));
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

#[test]
fn capabilities_match_litellm() {
    assert_eq!(
        CodexHarnessConfig.capabilities(),
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([PermissionMode::ReadOnly, PermissionMode::Full]),
        }
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
        harness: Harness::Codex,
        sandbox,
        session_id: "session-1".to_owned(),
        settings,
        outcome: TurnOutcome::default(),
    };
    (root, context)
}

#[rstest::rstest]
#[case::read_only_first_turn(PermissionMode::ReadOnly, None, false, &["--sandbox", "read-only"])]
#[case::full_first_turn(PermissionMode::Full, None, false, &["--sandbox", "workspace-write"])]
#[case::read_only_resumed(PermissionMode::ReadOnly, Some("thread-1"), false, &["-c", "sandbox_mode=\"read-only\""])]
#[case::full_resumed(PermissionMode::Full, Some("thread-1"), false, &["-c", "sandbox_mode=\"workspace-write\""])]
#[case::full_container(
    PermissionMode::Full,
    None,
    true,
    &["--dangerously-bypass-approvals-and-sandbox"]
)]
#[case::read_only_container_still_sandboxes(
    PermissionMode::ReadOnly,
    None,
    true,
    &["--sandbox", "read-only"]
)]
fn permission_args_match_mode_and_container(
    #[case] permissions: PermissionMode,
    #[case] native_session_id: Option<&str>,
    #[case] is_container: bool,
    #[case] expected: &[&str],
) {
    assert_eq!(
        super::permissions::permission_args(permissions, is_container, native_session_id),
        expected
    );
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

#[test]
fn setup_rejects_env_collision_with_managed_keys() {
    let (_root, mut context) = session_context();
    context.settings.options = Some(HarnessOptions::Codex(CodexOptions {
        reasoning_effort: None,
        web_search: false,
        config: JsonObject::new(),
        env: BTreeMap::from([("CODEX_HOME".to_owned(), "/elsewhere".to_owned())]),
    }));
    assert!(matches!(
        CodexHarnessConfig.transform_session_setup(&context, "/private"),
        Err(HarnessError::OptionsMismatch {
            given: "CodexOptions.env",
            harness: Harness::Codex,
            expected: "keys other than LITELLM_HARNESS_TOKEN or CODEX_HOME",
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
        thread_id: None,
        final_text: r#"{"answer":"yes"}"#.to_owned(),
        error: None,
        failed: false,
        started: BTreeSet::new(),
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
