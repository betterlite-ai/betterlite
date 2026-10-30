#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::{OpenCodeHarnessConfig, OpenCodeStreamState};
use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{HarnessOptions, OpenCodeOptions};
use crate::sandbox::Sandbox as _;
use crate::sandbox::local::LocalSandbox;
use crate::transformation::{BaseCLIHarnessConfig as _, BaseHarnessConfig as _, TurnInput};
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};

#[test]
fn opencode_validation_rejects_managed_keys() {
    let config: JsonObject = serde_json::from_str(r#"{"permission":{"edit":"allow"}}"#).unwrap();
    assert!(matches!(
        super::config::validate_user_config(&config),
        Err(HarnessError::OpenCodeManagedKeys { keys, .. }) if keys == ["permission".to_owned()]
    ));
}

#[test]
fn opencode_validation_rejects_agent_managed_keys() {
    let config: JsonObject =
        serde_json::from_str(r#"{"agent":{"build":{"model":"other"}}}"#).unwrap();
    assert!(matches!(
        super::config::validate_user_config(&config),
        Err(HarnessError::OpenCodeManagedKeys { section, agent, keys })
            if section == "agent" && agent.as_deref() == Some("build") && keys == ["model".to_owned()]
    ));
}

#[test]
fn opencode_validation_accepts_unmanaged_agent_fields() {
    let config: JsonObject =
        serde_json::from_str(r#"{"agent":{"build":{"description":"custom"}}}"#).unwrap();
    super::config::validate_user_config(&config).unwrap();
}

#[rstest::rstest]
#[case::read_only(PermissionMode::ReadOnly, &[], &[("bash", "deny"), ("edit", "deny"), ("webfetch", "deny")])]
#[case::edit(PermissionMode::Edit, &[], &[("bash", "deny"), ("edit", "allow"), ("webfetch", "allow")])]
#[case::full(PermissionMode::Full, &[], &[("*", "allow")])]
fn opencode_validation_permission_rules_match_mode(
    #[case] permissions: PermissionMode,
    #[case] disable_tools: &[&str],
    #[case] expected_pairs: &[(&str, &str)],
) {
    let disabled: Vec<String> = disable_tools
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();
    let rules = super::permissions::permission_rules(permissions, &disabled);
    let expected: BTreeMap<String, String> = expected_pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(rules, expected);
}

#[test]
fn opencode_validation_disable_tools_deny_wins_last() {
    let rules = super::permissions::permission_rules(
        PermissionMode::Edit,
        &["write".to_owned(), "bash".to_owned()],
    );
    assert_eq!(rules.get("edit").map(String::as_str), Some("deny"));
    assert_eq!(rules.get("bash").map(String::as_str), Some("deny"));
    assert_eq!(rules.get("webfetch").map(String::as_str), Some("allow"));
}

fn base_config_inputs<'cfg>(
    user_config: &'cfg JsonObject,
    disable_tools: &'cfg [String],
) -> super::config::ConfigInputs<'cfg> {
    super::config::ConfigInputs {
        model: "claude-x",
        base_url: "http://localhost:1234/v1".to_owned(),
        token_path: "/private/token".to_owned(),
        permissions: PermissionMode::Full,
        disable_tools,
        user_config,
        instructions_path: None,
        skills_path: None,
    }
}

#[test]
fn opencode_config_merges_underneath_managed_keys() {
    let user_config: JsonObject =
        serde_json::from_str(r#"{"instructions":["user.md"],"keep":true}"#).unwrap();
    let config =
        super::config::build_opencode_config(&base_config_inputs(&user_config, &[])).unwrap();
    assert_eq!(config.get("keep"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        config.get("model"),
        Some(&serde_json::Value::String("litellm/claude-x".to_owned()))
    );
    assert_eq!(
        config.get("autoupdate"),
        Some(&serde_json::Value::Bool(false))
    );
    let provider = config
        .get("provider")
        .and_then(|value| value.get("litellm"));
    assert!(provider.is_some());
}

#[test]
fn opencode_config_appends_extra_instructions_and_skills() {
    let user_config: JsonObject =
        serde_json::from_str(r#"{"instructions":["user.md"],"skills":{"paths":["user-skills"]}}"#)
            .unwrap();
    let disable_tools = Vec::new();
    let mut inputs = base_config_inputs(&user_config, &disable_tools);
    inputs.instructions_path = Some("/private/instructions.md".to_owned());
    inputs.skills_path = Some("/private/skills".to_owned());
    let config = super::config::build_opencode_config(&inputs).unwrap();
    let instructions = config["instructions"].as_array().unwrap();
    assert_eq!(instructions.len(), 2);
    assert_eq!(
        instructions[1],
        serde_json::Value::String("/private/instructions.md".to_owned())
    );
    let skill_paths = config["skills"]["paths"].as_array().unwrap();
    assert_eq!(skill_paths.len(), 2);
}

#[test]
fn opencode_config_rejects_managed_keys_before_merge() {
    let user_config: JsonObject = serde_json::from_str(r#"{"model":"other"}"#).unwrap();
    assert!(matches!(
        super::config::build_opencode_config(&base_config_inputs(&user_config, &[])),
        Err(HarnessError::OpenCodeManagedKeys { keys, .. }) if keys == ["model".to_owned()]
    ));
}

#[test]
fn opencode_config_build_instructions_appends_schema() {
    let (_root, mut context) = session_context();
    context.settings.instructions = Some("be terse".to_owned());
    context.settings.output_schema = Some(
        serde_json::from_str(r#"{"type":"object","properties":{"answer":{"type":"string"}}}"#)
            .unwrap(),
    );
    let instructions = super::config::build_instructions(&context)
        .unwrap()
        .unwrap();
    assert!(instructions.starts_with("be terse\n\n"));
    assert!(instructions.contains("JSON schema"));
    context.settings.output_schema = None;
    context.settings.instructions = None;
    assert!(
        super::config::build_instructions(&context)
            .unwrap()
            .is_none()
    );
}

#[test]
fn opencode_config_turn_prompt_repeats_schema() {
    let (_root, mut context) = session_context();
    context.settings.output_schema = Some(
        serde_json::from_str(r#"{"type":"object","properties":{"answer":{"type":"string"}}}"#)
            .unwrap(),
    );
    let prompt = super::config::turn_prompt(&context, "do work").unwrap();
    assert!(prompt.starts_with("do work\n\n"));
    assert!(prompt.contains("JSON schema"));
    context.settings.output_schema = None;
    assert_eq!(
        super::config::turn_prompt(&context, "do work").unwrap(),
        "do work"
    );
}

fn session_context() -> (tempfile::TempDir, SessionContext<LocalSandbox>) {
    let root = tempfile::tempdir().unwrap();
    let sandbox =
        LocalSandbox::new(root.path(), &BTreeMap::new(), root.path().to_path_buf()).unwrap();
    let settings = SessionSettings {
        model: Some("claude-x".to_owned()),
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
        harness: Harness::OpenCode,
        sandbox,
        session_id: "session-1".to_owned(),
        settings,
        outcome: TurnOutcome::default(),
    };
    (root, context)
}

#[test]
fn opencode_setup_isolates_token_and_xdg() {
    use secrecy::ExposeSecret as _;

    let (_root, context) = session_context();
    let setup = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert_eq!(
        setup.files.get("token").map(Vec::as_slice),
        Some(b"private-token".as_slice())
    );
    assert_eq!(setup.skills_dir.as_deref(), Some("skills"));
    assert_eq!(setup.persisted_dirs.first().unwrap().cache_path, "opencode");
    assert!(
        !setup.env["OPENCODE_CONFIG_CONTENT"]
            .expose_secret()
            .contains("private-token")
    );
    assert!(
        setup.env["OPENCODE_CONFIG_CONTENT"]
            .expose_secret()
            .contains("{file:/private/token}")
    );
}

#[rstest::rstest]
#[case::config("config")]
#[case::data("data")]
#[case::state("state")]
#[case::cache("cache")]
fn opencode_setup_isolates_xdg_subdir(#[case] sub: &str) {
    use secrecy::ExposeSecret as _;

    let (_root, context) = session_context();
    let setup = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let key = format!("XDG_{}_HOME", sub.to_uppercase());
    assert_eq!(
        setup.env[&key].expose_secret(),
        format!("/private/xdg/{sub}")
    );
}

#[test]
fn opencode_setup_writes_instructions_file_only_when_present() {
    let (_root, mut context) = session_context();
    let setup_without_instructions = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert!(
        !setup_without_instructions
            .files
            .contains_key("instructions.md")
    );
    context.settings.instructions = Some("be terse".to_owned());
    let setup_with_instructions = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    assert_eq!(
        setup_with_instructions
            .files
            .get("instructions.md")
            .map(Vec::as_slice),
        Some(b"be terse".as_slice())
    );
}

#[test]
fn opencode_setup_rejects_missing_gateway() {
    let (_root, mut context) = session_context();
    context.settings.gateway = None;
    assert!(matches!(
        OpenCodeHarnessConfig.transform_session_setup(&context, "/private"),
        Err(HarnessError::OptionsMismatch {
            given: "SessionSettings.gateway",
            harness: Harness::OpenCode,
            ..
        })
    ));
}

#[rstest::rstest]
#[case::first(None)]
#[case::resume(Some("session-xyz"))]
fn opencode_argv_matches_turn_shape(#[case] native_session_id: Option<&str>) {
    let (_root, context) = session_context();
    let setup = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let request = OpenCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "do work",
                native_session_id,
            },
        )
        .unwrap();
    assert_eq!(
        &request.argv[..8],
        &[
            "opencode".to_owned(),
            "run".to_owned(),
            "--pure".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
            "--thinking".to_owned(),
            "-m".to_owned(),
            "litellm/claude-x".to_owned(),
        ]
    );
    assert!(request.argv.iter().any(|arg| arg == "--agent"));
    assert!(request.argv.iter().any(|arg| arg == "build"));
    assert_eq!(request.stdin, "do work");
    assert_eq!(request.cwd.as_deref(), Some(context.sandbox.workdir()));
    let mut trailing = request.argv.iter().rev();
    let last = trailing.next().map(String::as_str);
    let second_to_last = trailing.next().map(String::as_str);
    if let Some(id) = native_session_id {
        assert_eq!(last, Some(id));
        assert_eq!(second_to_last, Some("--session"));
    } else {
        assert_eq!(last, Some("litellm-harness"));
        assert_eq!(second_to_last, Some("--title"));
    }
}

#[test]
fn opencode_argv_omits_agent_flag_when_empty() {
    let (_root, mut context) = session_context();
    context.settings.options = Some(HarnessOptions::OpenCode(OpenCodeOptions {
        agent: String::new(),
        config: JsonObject::new(),
        env: BTreeMap::new(),
    }));
    let setup = OpenCodeHarnessConfig
        .transform_session_setup(&context, "/private")
        .unwrap();
    let request = OpenCodeHarnessConfig
        .transform_turn_request(
            &context,
            TurnInput {
                setup: &setup,
                private_dir: "/private",
                prompt: "do work",
                native_session_id: None,
            },
        )
        .unwrap();
    assert!(!request.argv.iter().any(|arg| arg == "--agent"));
}

#[test]
fn opencode_parsing_text_and_reasoning_deltas() {
    let mut state = OpenCodeStreamState::default();
    let start: JsonObject = serde_json::from_str(r#"{"type":"step_start"}"#).unwrap();
    assert!(super::stream::transform_line(&start, &mut state).is_empty());
    let text: JsonObject =
        serde_json::from_str(r#"{"type":"text","part":{"text":"hello"}}"#).unwrap();
    let text_events = super::stream::transform_line(&text, &mut state);
    assert!(matches!(text_events.as_slice(), [Event::Text { delta }] if delta == "hello"));
    assert_eq!(state.final_text, "hello");
    let reasoning: JsonObject =
        serde_json::from_str(r#"{"type":"reasoning","part":{"text":"thinking"}}"#).unwrap();
    let reasoning_events = super::stream::transform_line(&reasoning, &mut state);
    assert!(
        matches!(reasoning_events.as_slice(), [Event::Reasoning { delta }] if delta == "thinking")
    );
}

#[test]
fn opencode_parsing_empty_text_emits_nothing() {
    let mut state = OpenCodeStreamState::default();
    let text: JsonObject = serde_json::from_str(r#"{"type":"text","part":{"text":""}}"#).unwrap();
    assert!(super::stream::transform_line(&text, &mut state).is_empty());
}

#[test]
fn opencode_parsing_tool_call_precedes_result() {
    let mut state = OpenCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"tool_use","part":{"tool":"edit","callID":"call-1","state":{"status":"completed","input":{"path":"a.py"},"output":"ok"}}}"#,
    )
    .unwrap();
    let events = super::stream::transform_line(&line, &mut state);
    assert!(matches!(
        events.first(),
        Some(Event::ToolCall { name, native_name, builtin: true, .. })
            if name == "edit" && native_name == "edit"
    ));
    assert!(matches!(
        events.get(1),
        Some(Event::ToolResult { output, is_error: false, .. }) if output == "ok"
    ));
}

#[test]
fn opencode_parsing_invalid_tool_is_error() {
    let mut state = OpenCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"tool_use","part":{"tool":"invalid","callID":"call-1","state":{"status":"completed","output":"no such tool"}}}"#,
    )
    .unwrap();
    let events = super::stream::transform_line(&line, &mut state);
    assert!(matches!(
        events.get(1),
        Some(Event::ToolResult { is_error: true, .. })
    ));
}

#[test]
fn opencode_parsing_tool_error_status_sets_is_error() {
    let mut state = OpenCodeStreamState::default();
    let line: JsonObject = serde_json::from_str(
        r#"{"type":"tool_use","part":{"tool":"bash","callID":"call-1","state":{"status":"error","error":"boom"}}}"#,
    )
    .unwrap();
    let events = super::stream::transform_line(&line, &mut state);
    assert!(matches!(
        events.get(1),
        Some(Event::ToolResult { output, is_error: true, .. }) if output == "boom"
    ));
}

#[test]
fn opencode_parsing_accumulates_error_messages() {
    let mut state = OpenCodeStreamState::default();
    let first: JsonObject =
        serde_json::from_str(r#"{"type":"error","error":"first problem"}"#).unwrap();
    assert!(super::stream::transform_line(&first, &mut state).is_empty());
    let second: JsonObject =
        serde_json::from_str(r#"{"type":"error","error":{"name":"second problem"}}"#).unwrap();
    assert!(super::stream::transform_line(&second, &mut state).is_empty());
    assert_eq!(
        state.error.as_deref(),
        Some("first problem\nsecond problem")
    );
}

#[test]
fn opencode_response_error_fails_even_at_exit_zero() {
    let (_root, context) = session_context();
    let state = OpenCodeStreamState {
        session_id: None,
        final_text: "partial".to_owned(),
        error: Some("opencode broke".to_owned()),
        step_texts: Vec::new(),
    };
    assert!(matches!(
        OpenCodeHarnessConfig.transform_turn_response(&context, &state, 0, &[]),
        Err(HarnessError::TurnFailed { .. })
    ));
}

#[test]
fn opencode_response_nonzero_exit_fails() {
    let (_root, context) = session_context();
    let state = OpenCodeStreamState::default();
    assert!(matches!(
        OpenCodeHarnessConfig.transform_turn_response(&context, &state, 1, &["boom".to_owned()]),
        Err(HarnessError::TurnFailed { .. })
    ));
}

#[test]
fn opencode_response_extracts_output_json_only_with_schema() {
    let (_root, mut context) = session_context();
    let state = OpenCodeStreamState {
        session_id: None,
        final_text: r#"here: {"answer":"yes"}"#.to_owned(),
        error: None,
        step_texts: Vec::new(),
    };
    context.settings.output_schema = Some(
        serde_json::from_str(r#"{"type":"object","properties":{"answer":{"type":"string"}}}"#)
            .unwrap(),
    );
    let response_with_schema = OpenCodeHarnessConfig
        .transform_turn_response(&context, &state, 0, &[])
        .unwrap();
    assert_eq!(
        response_with_schema.output_json.as_deref(),
        Some(r#"{"answer": "yes"}"#)
    );
    context.settings.output_schema = None;
    let response_without_schema = OpenCodeHarnessConfig
        .transform_turn_response(&context, &state, 0, &[])
        .unwrap();
    assert!(response_without_schema.output_json.is_none());
}

#[test]
fn opencode_response_capabilities_match_litellm() {
    assert_eq!(
        OpenCodeHarnessConfig.capabilities(),
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::ToolFiltering,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([
                PermissionMode::ReadOnly,
                PermissionMode::Edit,
                PermissionMode::Full,
            ]),
        }
    );
}
