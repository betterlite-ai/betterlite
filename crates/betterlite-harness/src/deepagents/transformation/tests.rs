#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{CodexOptions, DeepAgentsOptions, HarnessOptions};
use crate::transformation::BaseHarnessConfig as _;
use crate::types::{Capability, Harness, JsonObject, PermissionMode};

use super::config::{DeepAgentsHarnessConfig, recursion_limit};
use super::gateway::{chat_model_kwargs, gateway_headers};
use super::message::{
    Content, ContentBlock, Message, MessageKind, MessageUsage, ToolCall, content_text,
    final_ai_text, reasoning_text, stream_events, tool_call_event,
};
use super::tools::{
    AllowedDecision, blocked_tools, interrupt_config, native_tool_name, normalized_tool_name,
};
use super::update::{
    ActionRequest, HitlDecision, INTERRUPT_KEY, Interrupt, InterruptValue, TurnState, Update,
    UpdateEntry, decision, interrupts_in, structured_json, update_events,
};
use super::{
    BASE_RECURSION_LIMIT, DEEPAGENTS_STEPS_PER_TURN, DEFAULT_RECURSION_LIMIT, EVENT_NODES,
    INSTALL_HINT, MODEL_NODE, SKILLS_DIR,
};

fn json_object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn session_context() -> SessionContext<()> {
    let mut context = SessionContext {
        harness: Harness::DeepAgents,
        sandbox: (),
        session_id: "session".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    context.settings.model = Some("openai/gpt-4o".to_owned());
    context
}

fn plain_message(kind: MessageKind, content: Content) -> Message {
    Message {
        kind,
        content,
        reasoning_content: String::new(),
        tool_calls: Vec::new(),
        tool_call_id: String::new(),
        name: None,
        is_error: false,
        usage: MessageUsage::default(),
        reported_cost: None,
    }
}

fn tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_owned(),
        args: JsonObject::new(),
    }
}

fn ai_with_calls(calls: Vec<ToolCall>) -> Message {
    let mut message = plain_message(MessageKind::Ai, Content::Empty);
    message.tool_calls = calls;
    message
}

fn update_with(entries: Vec<(&str, UpdateEntry)>) -> Update {
    let mut update = Update::default();
    for (node, entry) in entries {
        update.entries.insert(node.to_owned(), entry);
    }
    update
}

fn interrupt(id: &str) -> Interrupt {
    Interrupt {
        id: id.to_owned(),
        value: InterruptValue {
            action_requests: Vec::new(),
        },
    }
}

#[rstest]
#[case::read("read", "read_file")]
#[case::write("write", "write_file")]
#[case::edit("edit", "edit_file")]
#[case::bash("bash", "execute")]
#[case::unmapped("grep", "grep")]
fn tool_names_map_between_vocabularies(#[case] normalized: &str, #[case] native: &str) {
    assert_eq!(native_tool_name(normalized), native);
    assert_eq!(normalized_tool_name(native), normalized);
}

#[test]
fn tool_name_mapping_passes_names_already_in_the_target_vocabulary_through() {
    assert_eq!(native_tool_name("read_file"), "read_file");
    assert_eq!(normalized_tool_name("read"), "read");
}

#[rstest]
#[case::read_only(
    PermissionMode::ReadOnly,
    &["delete", "edit_file", "execute", "read_file", "unknown", "write_file"]
)]
#[case::edit(PermissionMode::Edit, &["execute", "read_file", "unknown"])]
#[case::ask(PermissionMode::Ask, &["read_file", "unknown"])]
#[case::full(PermissionMode::Full, &["read_file", "unknown"])]
fn blocked_tools_follows_the_permission_mode(
    #[case] permissions: PermissionMode,
    #[case] expected: &[&str],
) {
    let disable_tools = vec!["read".to_owned(), "unknown".to_owned()];
    let blocked = blocked_tools(permissions, &disable_tools);
    let kept: Vec<&str> = blocked.iter().map(String::as_str).collect();
    assert_eq!(kept, expected);
}

#[rstest]
#[case::read_only(PermissionMode::ReadOnly)]
#[case::edit(PermissionMode::Edit)]
#[case::full(PermissionMode::Full)]
fn interrupt_config_is_absent_outside_ask_mode(#[case] permissions: PermissionMode) {
    assert!(interrupt_config(permissions, &BTreeSet::new()).is_none());
}

#[test]
fn interrupt_config_lists_every_mutating_tool_of_ask_mode() {
    let config = interrupt_config(PermissionMode::Ask, &BTreeSet::new()).unwrap();
    let tools: Vec<&str> = config.keys().map(String::as_str).collect();
    assert_eq!(tools, ["delete", "edit_file", "execute", "write_file"]);
    assert!(config.values().all(|policy| {
        policy.allowed_decisions == [AllowedDecision::Approve, AllowedDecision::Reject]
    }));
}

#[test]
fn interrupt_config_matches_the_langgraph_interrupt_on_shape() {
    let config = interrupt_config(PermissionMode::Ask, &BTreeSet::new()).unwrap();
    assert_eq!(
        serde_json::to_string(&config).unwrap(),
        r#"{"delete":{"allowed_decisions":["approve","reject"]},"edit_file":{"allowed_decisions":["approve","reject"]},"execute":{"allowed_decisions":["approve","reject"]},"write_file":{"allowed_decisions":["approve","reject"]}}"#
    );
}

#[test]
fn interrupt_config_skips_blocked_tools() {
    let blocked = BTreeSet::from(["execute".to_owned(), "write_file".to_owned()]);
    let config = interrupt_config(PermissionMode::Ask, &blocked).unwrap();
    let tools: Vec<&str> = config.keys().map(String::as_str).collect();
    assert_eq!(tools, ["delete", "edit_file"]);
}

#[test]
fn recursion_limit_prefers_the_options_value() {
    let options = HarnessOptions::DeepAgents(DeepAgentsOptions {
        subagents: Vec::new(),
        recursion_limit: Some(77),
    });
    let mut context = session_context();
    context.settings.max_turns = Some(3);
    context.settings.options = Some(options);
    assert_eq!(recursion_limit(&context), 77);
}

#[rstest]
#[case::one_turn(1, 31)]
#[case::three_turns(3, 43)]
#[case::no_turns(0, 25)]
fn recursion_limit_scales_max_turns_by_graph_steps(#[case] max_turns: u32, #[case] expected: u32) {
    let mut context = session_context();
    context.settings.max_turns = Some(max_turns);
    assert_eq!(recursion_limit(&context), expected);
}

#[test]
fn recursion_limit_defaults_without_max_turns() {
    assert_eq!(recursion_limit(&session_context()), DEFAULT_RECURSION_LIMIT);
}

#[test]
fn recursion_limit_saturates_at_the_u32_maximum() {
    let mut context = session_context();
    context.settings.max_turns = Some(u32::MAX);
    assert_eq!(recursion_limit(&context), u32::MAX);
}

#[test]
fn recursion_limit_ignores_another_harness_options() {
    let mut context = session_context();
    context.settings.max_turns = Some(1);
    context.settings.options = Some(HarnessOptions::Codex(CodexOptions::default()));
    assert_eq!(
        recursion_limit(&context),
        BASE_RECURSION_LIMIT + DEEPAGENTS_STEPS_PER_TURN
    );
}

#[test]
fn content_text_joins_text_blocks_and_ignores_reasoning_and_other_blocks() {
    let content = Content::Blocks(vec![
        ContentBlock::Text("one ".to_owned()),
        ContentBlock::Reasoning("hidden".to_owned()),
        ContentBlock::Other(json_object(r#"{"type":"image"}"#)),
        ContentBlock::Text("two".to_owned()),
    ]);
    assert_eq!(content_text(&content), "one two");
}

#[test]
fn content_text_handles_text_and_empty_content() {
    assert_eq!(content_text(&Content::Text("plain".to_owned())), "plain");
    assert_eq!(content_text(&Content::Empty), "");
    assert_eq!(content_text(&Content::Blocks(Vec::new())), "");
}

#[test]
fn reasoning_text_prefers_reasoning_content_over_blocks() {
    let mut message = plain_message(
        MessageKind::Ai,
        Content::Blocks(vec![ContentBlock::Reasoning("block".to_owned())]),
    );
    message.reasoning_content = "delta".to_owned();
    assert_eq!(reasoning_text(&message), "delta");
}

#[test]
fn reasoning_text_falls_back_to_reasoning_blocks() {
    let mut message = plain_message(
        MessageKind::Ai,
        Content::Blocks(vec![
            ContentBlock::Reasoning("first".to_owned()),
            ContentBlock::Text("ignored".to_owned()),
            ContentBlock::Reasoning("second".to_owned()),
        ]),
    );
    message.reasoning_content = String::new();
    assert_eq!(reasoning_text(&message), "firstsecond");
}

#[test]
fn reasoning_text_is_empty_for_plain_text_messages() {
    let message = plain_message(MessageKind::Ai, Content::Text("plain".to_owned()));
    assert_eq!(reasoning_text(&message), "");
}

#[rstest]
#[case::chunk(MessageKind::AiMessageChunk)]
#[case::message(MessageKind::Ai)]
fn stream_events_yields_reasoning_before_text(#[case] kind: MessageKind) {
    let mut message = plain_message(kind, Content::Text("hello".to_owned()));
    message.reasoning_content = "think".to_owned();
    let events = stream_events(&message);
    assert_eq!(events.len(), 2);
    let Event::Reasoning { delta } = &events[0] else {
        panic!("expected a reasoning event");
    };
    assert_eq!(delta, "think");
    let Event::Text { delta } = &events[1] else {
        panic!("expected a text event");
    };
    assert_eq!(delta, "hello");
}

#[test]
fn stream_events_skips_empty_reasoning_and_text() {
    assert!(stream_events(&plain_message(MessageKind::Ai, Content::Empty)).is_empty());
}

#[test]
fn stream_events_ignores_messages_that_are_not_ai() {
    let human = plain_message(MessageKind::Human, Content::Text("hi".to_owned()));
    assert!(stream_events(&human).is_empty());
    let tool = plain_message(MessageKind::Tool, Content::Text("hi".to_owned()));
    assert!(stream_events(&tool).is_empty());
}

#[test]
fn tool_call_event_normalizes_the_name_and_marks_builtins() {
    let call = ToolCall {
        id: "call-1".to_owned(),
        name: "read_file".to_owned(),
        args: json_object(r#"{"path":"/src/a.py"}"#),
    };
    let Event::ToolCall {
        id,
        name,
        native_name,
        input,
        builtin,
    } = tool_call_event(&call)
    else {
        panic!("expected a tool call event");
    };
    assert_eq!(
        (id.as_str(), name.as_str(), native_name.as_str()),
        ("call-1", "read", "read_file")
    );
    assert_eq!(input, call.args);
    assert!(builtin);
}

#[test]
fn tool_call_event_passes_unmapped_names_through() {
    let call = tool_call("custom_tool");
    let Event::ToolCall { name, builtin, .. } = tool_call_event(&call) else {
        panic!("expected a tool call event");
    };
    assert_eq!(name, "custom_tool");
    assert!(!builtin);
}

#[test]
fn final_ai_text_takes_the_last_non_empty_ai_message() {
    let messages = vec![
        plain_message(MessageKind::Human, Content::Text("question".to_owned())),
        plain_message(MessageKind::Ai, Content::Text("answer".to_owned())),
        plain_message(
            MessageKind::AiMessageChunk,
            Content::Text("chunk".to_owned()),
        ),
        plain_message(MessageKind::Ai, Content::Empty),
    ];
    assert_eq!(final_ai_text(&messages), "answer");
}

#[test]
fn final_ai_text_is_empty_without_ai_text() {
    assert_eq!(final_ai_text(&[]), "");
    let ai = plain_message(MessageKind::Ai, Content::Empty);
    assert_eq!(final_ai_text(&[ai]), "");
}

#[test]
fn update_events_reads_only_the_model_and_tools_nodes() {
    let update = update_with(vec![
        (
            "model",
            UpdateEntry::Messages(vec![ai_with_calls(vec![tool_call("read_file")])]),
        ),
        (
            "tools",
            UpdateEntry::Messages(vec![plain_message(
                MessageKind::Tool,
                Content::Text("out".to_owned()),
            )]),
        ),
        (
            "summary",
            UpdateEntry::Messages(vec![ai_with_calls(vec![tool_call("write_file")])]),
        ),
        (
            "__interrupt__",
            UpdateEntry::Interrupts(vec![interrupt("int-1")]),
        ),
    ]);
    let events = update_events(&update, &BTreeSet::new());
    assert_eq!(events.len(), 2);
    let Event::ToolCall { native_name, .. } = &events[0] else {
        panic!("expected the model node tool call");
    };
    assert_eq!(native_name, "read_file");
    let Event::ToolResult { output, .. } = &events[1] else {
        panic!("expected the tools node result");
    };
    assert_eq!(output, "out");
}

#[test]
fn update_events_skips_tools_named_in_skip_tools() {
    let mut result = plain_message(MessageKind::Tool, Content::Text("done".to_owned()));
    result.name = Some("finish".to_owned());
    let update = update_with(vec![
        (
            "model",
            UpdateEntry::Messages(vec![ai_with_calls(vec![
                tool_call("read_file"),
                tool_call("finish"),
            ])]),
        ),
        ("tools", UpdateEntry::Messages(vec![result])),
    ]);
    let skip_tools = BTreeSet::from(["finish".to_owned()]);
    let events = update_events(&update, &skip_tools);
    assert_eq!(events.len(), 1);
    let Event::ToolCall { native_name, .. } = &events[0] else {
        panic!("expected the unskipped tool call");
    };
    assert_eq!(native_name, "read_file");
}

#[test]
fn update_events_builds_tool_results_with_status_and_call_id() {
    let mut message = plain_message(MessageKind::Tool, Content::Text("boom".to_owned()));
    message.tool_call_id = "call-9".to_owned();
    message.is_error = true;
    let update = update_with(vec![("tools", UpdateEntry::Messages(vec![message]))]);
    let events = update_events(&update, &BTreeSet::new());
    let Event::ToolResult {
        id,
        output,
        is_error,
    } = &events[0]
    else {
        panic!("expected a tool result");
    };
    assert_eq!(
        (id.as_str(), output.as_str(), *is_error),
        ("call-9", "boom", true)
    );
}

#[test]
fn interrupts_in_collects_interrupt_entries() {
    let expected = interrupt("int-1");
    let update = update_with(vec![
        (
            "model",
            UpdateEntry::Messages(vec![plain_message(MessageKind::Ai, Content::Empty)]),
        ),
        (
            "__interrupt__",
            UpdateEntry::Interrupts(vec![expected.clone()]),
        ),
    ]);
    assert_eq!(interrupts_in(&update), [expected]);
}

#[test]
fn turn_state_accumulates_interrupts_across_updates() {
    let first = update_with(vec![(
        "__interrupt__",
        UpdateEntry::Interrupts(vec![interrupt("a")]),
    )]);
    let second = update_with(vec![(
        "__interrupt__",
        UpdateEntry::Interrupts(vec![interrupt("b"), interrupt("c")]),
    )]);
    let mut state = TurnState::default();
    state.interrupts.extend(interrupts_in(&first));
    state.interrupts.extend(interrupts_in(&second));
    let ids: Vec<&str> = state
        .interrupts
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn interrupt_values_carry_their_action_requests() {
    let value = InterruptValue {
        action_requests: vec![ActionRequest {
            name: "write_file".to_owned(),
            args: json_object(r#"{"path":"a"}"#),
        }],
    };
    assert_eq!(value.action_requests.len(), 1);
    assert_eq!(value.action_requests[0].name, "write_file");
    assert_eq!(
        value.action_requests[0].args,
        json_object(r#"{"path":"a"}"#)
    );
}

#[test]
fn an_interrupt_value_without_action_requests_is_empty() {
    assert!(InterruptValue::default().action_requests.is_empty());
}

#[test]
fn decision_approves_when_allowed() {
    assert_eq!(decision(true, "ignored"), HitlDecision::Approve);
}

#[test]
fn decision_rejects_with_the_given_reason() {
    assert_eq!(
        decision(false, "too risky"),
        HitlDecision::Reject {
            message: "too risky".to_owned()
        }
    );
}

#[test]
fn decision_rejects_with_the_default_message_for_an_empty_reason() {
    assert_eq!(
        decision(false, ""),
        HitlDecision::Reject {
            message: "The user denied this tool call.".to_owned()
        }
    );
}

#[test]
fn hitl_decisions_serialize_to_the_resume_payload() {
    assert_eq!(
        serde_json::to_string(&decision(true, "")).unwrap(),
        r#"{"type":"approve"}"#
    );
    assert_eq!(
        serde_json::to_string(&decision(false, "no")).unwrap(),
        r#"{"type":"reject","message":"no"}"#
    );
}

#[test]
fn structured_json_serializes_objects_and_passes_none_through() {
    let object = json_object(r#"{"answer":"42"}"#);
    assert_eq!(
        structured_json(Some(&object)).unwrap(),
        Some(r#"{"answer":"42"}"#.to_owned())
    );
    assert_eq!(structured_json(None).unwrap(), None);
}

#[test]
fn gateway_headers_carry_the_attribution_tags() {
    let headers = gateway_headers(&session_context()).unwrap();
    assert_eq!(
        headers.get("x-litellm-tags").map(String::as_str),
        Some("harness,deepagents")
    );
    assert!(!headers.contains_key("x-litellm-spend-logs-metadata"));
}

#[test]
fn gateway_headers_carry_the_metadata_json_when_metadata_is_set() {
    let mut context = session_context();
    context.settings.metadata = json_object(r#"{"user":"alice"}"#);
    let headers = gateway_headers(&context).unwrap();
    assert_eq!(
        headers
            .get("x-litellm-spend-logs-metadata")
            .map(String::as_str),
        Some(r#"{"user":"alice"}"#)
    );
}

#[rstest]
#[case::claude_code(Harness::ClaudeCode, "harness,claude_code")]
#[case::codex(Harness::Codex, "harness,codex")]
#[case::opencode(Harness::OpenCode, "harness,opencode")]
#[case::deepagents(Harness::DeepAgents, "harness,deepagents")]
fn gateway_headers_name_every_harness(#[case] harness: Harness, #[case] tags: &str) {
    let mut context = session_context();
    context.harness = harness;
    let headers = gateway_headers(&context).unwrap();
    assert_eq!(
        headers.get("x-litellm-tags").map(String::as_str),
        Some(tags)
    );
}

#[test]
fn chat_model_kwargs_use_the_gateway_when_one_is_configured() {
    let mut context = session_context();
    context.settings.gateway = Some(GatewayTarget {
        api_base: "http://proxy:4000".to_owned(),
        api_key: SecretString::from("proxy-key"),
    });
    let kwargs = chat_model_kwargs(&context).unwrap();
    assert_eq!(kwargs.model, "litellm_proxy/openai/gpt-4o");
    assert_eq!(kwargs.api_base.as_deref(), Some("http://proxy:4000"));
    assert_eq!(
        kwargs.api_key.as_ref().unwrap().expose_secret(),
        "proxy-key"
    );
    let headers = kwargs.extra_headers.unwrap();
    assert_eq!(
        headers.get("x-litellm-tags").map(String::as_str),
        Some("harness,deepagents")
    );
}

#[test]
fn chat_model_kwargs_use_the_sdk_without_a_gateway() {
    let mut context = session_context();
    context.settings.api_base = Some("http://sdk".to_owned());
    context.settings.api_key = Some(SecretString::from("sdk-key"));
    let kwargs = chat_model_kwargs(&context).unwrap();
    assert_eq!(kwargs.model, "openai/gpt-4o");
    assert_eq!(kwargs.api_base.as_deref(), Some("http://sdk"));
    assert_eq!(kwargs.api_key.as_ref().unwrap().expose_secret(), "sdk-key");
    assert!(kwargs.extra_headers.is_none());
}

#[test]
fn chat_model_kwargs_require_a_model() {
    let mut context = session_context();
    context.settings.model = None;
    assert!(matches!(
        chat_model_kwargs(&context),
        Err(HarnessError::OptionsMismatch {
            given: "SessionSettings.model",
            ..
        })
    ));
}

#[test]
fn validate_environment_requires_a_model() {
    let mut context = session_context();
    context.settings.model = None;
    let error = DeepAgentsHarnessConfig
        .validate_environment(&context)
        .unwrap_err();
    assert!(matches!(
        error,
        HarnessError::OptionsMismatch {
            given: "SessionSettings.model",
            ..
        }
    ));
}

#[test]
fn validate_environment_rejects_an_empty_model() {
    let mut context = session_context();
    context.settings.model = Some(String::new());
    assert!(
        DeepAgentsHarnessConfig
            .validate_environment(&context)
            .is_err()
    );
}

#[test]
fn validate_environment_accepts_a_model() {
    assert!(
        DeepAgentsHarnessConfig
            .validate_environment(&session_context())
            .is_ok()
    );
}

#[test]
fn validate_environment_rejects_another_harness_options() {
    let mut context = session_context();
    context.settings.options = Some(HarnessOptions::Codex(CodexOptions::default()));
    let error = DeepAgentsHarnessConfig
        .validate_environment(&context)
        .unwrap_err();
    assert!(matches!(
        error,
        HarnessError::OptionsMismatch {
            given: "CodexOptions",
            ..
        }
    ));
}

#[test]
fn config_declares_every_deepagents_capability() {
    let capabilities = DeepAgentsHarnessConfig.capabilities();
    assert_eq!(
        capabilities.supported,
        BTreeSet::from([
            Capability::StructuredOutput,
            Capability::ToolApproval,
            Capability::ToolFiltering,
            Capability::History,
            Capability::CustomTools,
            Capability::Skills,
            Capability::Resume,
        ])
    );
    assert_eq!(
        capabilities.permission_modes,
        BTreeSet::from([
            PermissionMode::ReadOnly,
            PermissionMode::Ask,
            PermissionMode::Edit,
            PermissionMode::Full,
        ])
    );
}

#[test]
fn config_is_an_in_process_harness() {
    assert_eq!(DeepAgentsHarnessConfig::HARNESS, Harness::DeepAgents);
    const { assert!(!DeepAgentsHarnessConfig::USES_MODEL_ENDPOINT) };
}

#[test]
fn constants_match_litellm() {
    assert_eq!(
        INSTALL_HINT,
        "Deep Agents is not installed. Run: pip install deepagents langchain-litellm"
    );
    assert_eq!(SKILLS_DIR, ".deepagents/skills");
    assert_eq!(DEEPAGENTS_STEPS_PER_TURN, 6);
    assert_eq!(BASE_RECURSION_LIMIT, 25);
    assert_eq!(DEFAULT_RECURSION_LIMIT, 1000);
    assert_eq!(MODEL_NODE, "model");
    assert_eq!(EVENT_NODES, ["model", "tools"]);
}

#[test]
fn interrupt_key_is_the_langgraph_interrupt_key() {
    assert_eq!(INTERRUPT_KEY, "__interrupt__");
}

#[test]
fn json_object_keeps_the_args_the_model_sent() {
    let args = json_object(r#"{"limit":3,"query":"x"}"#);
    let call = ToolCall {
        id: "call-2".to_owned(),
        name: "grep".to_owned(),
        args: args.clone(),
    };
    let Event::ToolCall { input, .. } = tool_call_event(&call) else {
        panic!("expected a tool call event");
    };
    assert_eq!(input, args);
}
