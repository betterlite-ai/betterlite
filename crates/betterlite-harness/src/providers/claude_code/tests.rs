#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use secrecy::ExposeSecret as _;

use super::ClaudeCodeHarnessConfig;
use super::config::NATIVE_TO_NORMALIZED;
use super::stream::ClaudeCodeStreamState;
use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{ClaudeCodeOptions, HarnessOptions};
use crate::sandbox::local::LocalSandbox;
use crate::transformation::{
    BaseCLIHarnessConfig as _, BaseHarnessConfig as _, PersistedDir, TurnInput,
};
use crate::types::{Capability, Harness, JsonObject, PermissionMode};

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
    assert_eq!(
        error.to_string(),
        "ClaudeCodeOptions.env may not set ANTHROPIC_API_KEY; LiteLLM manages it"
    );
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
    assert_eq!(
        message,
        "ClaudeCodeOptions.config may not set model, permissions; use the matching agent() argument (model=, permissions=) instead"
    );
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

#[rstest::rstest]
#[case::mixed_content("mixed-content")]
#[case::tool_output("tool-output")]
#[case::unicode_result("unicode-result")]
fn claude_code_fuzz_corpus_replays_every_seed(#[case] seed: &str) {
    let directory = format!("{}/fuzz/corpus/claude_code", env!("CARGO_MANIFEST_DIR"));
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);
    let bytes = fs::read(format!("{directory}/{seed}")).unwrap();
    if let Ok(line) = serde_json::from_slice::<JsonObject>(&bytes) {
        let mut state = ClaudeCodeStreamState::default();
        drop(super::stream::transform(&line, &mut state));
    }
}

#[test]
fn claude_code_response_prefers_result_text_and_structured_output() {
    let mut state = ClaudeCodeStreamState::default();
    state.text_parts.push("stream text".to_owned());
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

#[test]
fn claude_code_response_empty_error_uses_fallback() {
    let mut state = ClaudeCodeStreamState::default();
    let line = serde_json::from_str(r#"{"type":"result","is_error":true,"errors":[""]}"#).unwrap();
    super::stream::transform(&line, &mut state);
    assert_eq!(
        super::stream::turn_error(&state, 0, &[]).as_deref(),
        Some("claude exited with code 0: unknown error")
    );
}

#[test]
fn claude_code_validation_rejects_empty_endpoint_token() {
    let (_root, mut context) = session_context();
    context.settings.gateway.as_mut().unwrap().api_key = secrecy::SecretString::from("");
    assert!(matches!(
        ClaudeCodeHarnessConfig.transform_session_setup(&context, "/private"),
        Err(HarnessError::ClaudeCodeEndpoint)
    ));
}

#[test]
fn claude_code_argv_omits_empty_model_and_resume() {
    let (_root, mut context) = session_context();
    context.settings.model = Some(String::new());
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert!(!setup.env.contains_key("ANTHROPIC_MODEL"));
    assert!(!setup.env.contains_key("ANTHROPIC_SMALL_FAST_MODEL"));
    let request = ClaudeCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "hi",
                native_session_id: Some(""),
            },
        )
        .unwrap();
    assert!(
        !request
            .argv
            .iter()
            .any(|arg| arg == "--model" || arg == "--resume")
    );
}

#[rstest::rstest]
#[case::object(
    r#"{"items":[1,2],"word":"\u00e9"}"#,
    "{\"items\": [1, 2], \"word\": \"\u{e9}\"}"
)]
#[case::number(r#"{"n":1e-7}"#, r#"{"n": 1e-07}"#)]
#[case::blocks(
    r#"[{"data":"\u00e9","type":"image"},"tail"]"#,
    "{\"data\": \"\u{e9}\", \"type\": \"image\"}\ntail"
)]
fn claude_code_parsing_unicode_tool_output(#[case] content: &str, #[case] expected: &str) {
    let line = serde_json::from_str(&format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":{content}}}]}}}}"#)).unwrap();
    let mut state = ClaudeCodeStreamState::default();
    let events = super::stream::transform(&line, &mut state);
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events.first(), Some(Event::ToolResult { id, output, is_error }) if id.is_empty() && output == expected && !is_error)
    );
}

#[rstest::rstest]
#[case::base_url("ANTHROPIC_BASE_URL", true)]
#[case::auth_token("ANTHROPIC_AUTH_TOKEN", true)]
#[case::api_key("ANTHROPIC_API_KEY", true)]
#[case::model_env("ANTHROPIC_MODEL", true)]
#[case::small_model("ANTHROPIC_SMALL_FAST_MODEL", true)]
#[case::private_dir("CLAUDE_CONFIG_DIR", true)]
#[case::env("env", false)]
#[case::api_key_helper("apiKeyHelper", false)]
#[case::model_config("model", false)]
#[case::permissions("permissions", false)]
#[case::aws_refresh("awsAuthRefresh", false)]
#[case::aws_export("awsCredentialExport", false)]
#[case::login("forceLoginMethod", false)]
fn claude_code_validation_managed_keys(#[case] key: &str, #[case] env: bool) {
    let mut options = ClaudeCodeOptions::default();
    if env {
        options.env.insert(key.to_owned(), String::new());
    } else {
        options
            .config
            .insert(key.to_owned(), serde_json::Value::Null);
    }
    assert!(
        super::config::validate_options(&options)
            .unwrap_err()
            .to_string()
            .contains(key)
    );
}

#[rstest::rstest]
#[case::read_only(PermissionMode::ReadOnly, "plan")]
#[case::ask(PermissionMode::Ask, "default")]
#[case::edit(PermissionMode::Edit, "acceptEdits")]
#[case::full(PermissionMode::Full, "bypassPermissions")]
fn claude_code_argv_permissions(#[case] permission: PermissionMode, #[case] native: &str) {
    let (_root, mut context) = session_context();
    context.settings.permissions = permission;
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let request = ClaudeCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "hello",
                native_session_id: Some("prior"),
            },
        )
        .unwrap();
    assert_eq!(
        request.argv,
        [
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "text",
            "--permission-mode",
            native,
            "--setting-sources",
            "user",
            "--resume",
            "prior"
        ]
    );
    assert_eq!(request.stdin, "hello");
    assert_eq!(request.cwd, None);
}

#[test]
fn claude_code_argv_private_environment() {
    let (_root, mut context) = session_context();
    let mut options = ClaudeCodeOptions::default();
    options.env.insert("X".to_owned(), "custom".to_owned());
    options
        .env
        .insert("DISABLE_TELEMETRY".to_owned(), "0".to_owned());
    context.settings.options = Some(HarnessOptions::ClaudeCode(options));
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let exposed: BTreeMap<&str, &str> = setup
        .env
        .iter()
        .map(|(key, value)| (key.as_str(), value.expose_secret()))
        .collect();
    assert_eq!(
        exposed,
        BTreeMap::from([
            ("X", "custom"),
            ("DISABLE_TELEMETRY", "1"),
            ("DISABLE_ERROR_REPORTING", "1"),
            ("DISABLE_AUTOUPDATER", "1"),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
            ("ANTHROPIC_BASE_URL", "http://localhost:1234/"),
            ("ANTHROPIC_AUTH_TOKEN", "private-token"),
            ("ANTHROPIC_API_KEY", ""),
            ("CLAUDE_CONFIG_DIR", "/private"),
        ])
    );
    assert!(setup.files.is_empty());
    assert_eq!(setup.skills_dir.as_deref(), Some("skills"));
    assert_eq!(
        setup.persisted_dirs,
        vec![PersistedDir {
            private_path: "projects".to_owned(),
            cache_path: "claude_code/projects".to_owned(),
        }]
    );
}

#[test]
fn claude_code_capabilities_and_installation() {
    assert_eq!(ClaudeCodeHarnessConfig.get_binary(), "claude");
    assert_eq!(
        ClaudeCodeHarnessConfig.get_install_hint(),
        "npm install -g @anthropic-ai/claude-code"
    );
    let capabilities = ClaudeCodeHarnessConfig.capabilities();
    assert_eq!(
        capabilities.supported,
        BTreeSet::from([
            Capability::StructuredOutput,
            Capability::ToolFiltering,
            Capability::Skills,
            Capability::Resume,
        ])
    );
    assert_eq!(
        capabilities.permission_modes,
        BTreeSet::from([
            PermissionMode::ReadOnly,
            PermissionMode::Edit,
            PermissionMode::Full
        ])
    );
}

#[rstest::rstest]
#[case::exit_failure(1_i32, r#"{"type":"result"}"#, Some("unknown error"))]
#[case::runtime_error(
    0_i32,
    r#"{"type":"result","is_error":true,"result":"failed"}"#,
    Some("failed")
)]
#[case::errors(
    0_i32,
    r#"{"type":"result","is_error":true,"errors":["one","two"]}"#,
    Some("one; two")
)]
#[case::success(0_i32, r#"{"type":"result"}"#, None)]
#[case::no_result(0_i32, r#"{"type":"unknown"}"#, Some("no result event"))]
fn claude_code_response_status(
    #[case] exit_code: i32,
    #[case] input: &str,
    #[case] reason: Option<&str>,
) {
    let (_root, context) = session_context();
    let mut state = ClaudeCodeStreamState::default();
    super::stream::transform(&serde_json::from_str(input).unwrap(), &mut state);
    let response = ClaudeCodeHarnessConfig.transform_turn_response(
        &context,
        &state,
        exit_code,
        &[" ".to_owned(), "diagnostic".to_owned()],
    );
    if let Some(message) = reason {
        let HarnessError::TurnFailed { reason: actual } = response.unwrap_err() else {
            panic!("wrong error");
        };
        assert_eq!(
            actual,
            format!("claude exited with code {exit_code}: {message}\nstderr:\ndiagnostic")
        );
    } else {
        response.unwrap();
    }
}

#[test]
fn claude_code_response_schema_controls_output() {
    let (_root, mut context) = session_context();
    let mut state = ClaudeCodeStreamState::default();
    super::stream::transform(
        &serde_json::from_str(
            r#"{"type":"assistant","message":{"content":"prefix {\"ok\":true}"}}"#,
        )
        .unwrap(),
        &mut state,
    );
    super::stream::transform(
        &serde_json::from_str(r#"{"type":"result","structured_output":[]}"#).unwrap(),
        &mut state,
    );
    assert_eq!(state.final_text(), "prefix {\"ok\":true}");
    assert_eq!(
        ClaudeCodeHarnessConfig
            .transform_turn_response(&context, &state, 0, &[])
            .unwrap()
            .output_json,
        None
    );
    context.settings.output_schema = Some(JsonObject::new());
    assert_eq!(
        ClaudeCodeHarnessConfig
            .transform_turn_response(&context, &state, 0, &[])
            .unwrap()
            .output_json
            .as_deref(),
        Some(r#"{"ok": true}"#)
    );
}

#[test]
fn claude_code_argv_settings_and_schema_prompt() {
    let (_root, mut context) = session_context();
    context.settings.options = Some(HarnessOptions::ClaudeCode(ClaudeCodeOptions {
        config: serde_json::from_str(r#"{"cleanupPeriodDays":1,"language":"\u00e9"}"#).unwrap(),
        env: BTreeMap::new(),
    }));
    context.settings.instructions = Some("Base.".to_owned());
    context.settings.output_schema = Some(serde_json::from_str(r#"{"type":"object"}"#).unwrap());
    context.settings.max_turns = Some(0);
    let setup = ClaudeCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let request = ClaudeCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "hello",
                native_session_id: None,
            },
        )
        .unwrap();
    assert!(request.argv.windows(2).any(|pair| pair
        == [
            "--settings",
            r#"{"cleanupPeriodDays": 1, "language": "\u00e9"}"#
        ]));
    assert!(
        request
            .argv
            .windows(2)
            .any(|pair| pair == ["--max-turns", "0"])
    );
    let expected = "Base.\n\nWhen you have finished the task, end your final reply with only a single JSON object (no code fences, no prose after it) that matches this JSON schema:\n{\"type\": \"object\"}";
    assert!(
        request
            .argv
            .windows(2)
            .any(|pair| pair == ["--append-system-prompt", expected])
    );
}

#[rstest::rstest]
#[case::subagent_user(r#"{"type":"user","parent_tool_use_id":"parent","message":{"content":[{"type":"tool_result","content":"hidden"}]}}"#)]
#[case::synthetic(r#"{"type":"assistant","message":{"model":"<synthetic>","content":"hidden"}}"#)]
#[case::empty_blocks(r#"{"type":"assistant","message":{"content":[{"type":"text","text":""},{"type":"thinking","thinking":""},{"type":"future"}]}}"#)]
#[case::unknown(r#"{"type":"future"}"#)]
fn claude_code_parsing_suppressed_events(#[case] source: &str) {
    let mut state = ClaudeCodeStreamState::default();
    let line = serde_json::from_str(source).unwrap();
    assert!(super::stream::transform(&line, &mut state).is_empty());
    assert!(state.text_parts.is_empty());
    assert!(!state.result_seen);
}

#[test]
fn claude_code_parsing_native_names_and_session_precedence() {
    let mut state = ClaudeCodeStreamState::default();
    super::stream::transform(
        &serde_json::from_str(r#"{"type":"system","subtype":"init","session_id":"initial"}"#)
            .unwrap(),
        &mut state,
    );
    assert_eq!(
        ClaudeCodeHarnessConfig
            .get_native_session_id(&state)
            .as_deref(),
        Some("initial")
    );
    let line = serde_json::from_str(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__x__y","id":"t"},{"type":"tool_use","name":"MultiEdit","id":"u"}]}}"#).unwrap();
    let events = super::stream::transform(&line, &mut state);
    assert!(
        matches!(events.first(), Some(Event::ToolCall { name, native_name, builtin: false, .. }) if name == "mcp__x__y" && native_name == name)
    );
    assert!(
        matches!(events.get(1), Some(Event::ToolCall { name, native_name, builtin: true, .. }) if name == "edit" && native_name == "MultiEdit")
    );
    super::stream::transform(
        &serde_json::from_str(r#"{"type":"result","session_id":"final","result":""}"#).unwrap(),
        &mut state,
    );
    assert_eq!(
        ClaudeCodeHarnessConfig
            .get_native_session_id(&state)
            .as_deref(),
        Some("final")
    );
    assert_eq!(state.final_text(), "");
    assert_eq!(
        super::config::native_tool_names(&[
            "edit".to_owned(),
            "Task".to_owned(),
            "edit".to_owned()
        ]),
        ["Edit", "MultiEdit", "Task"]
    );
}
