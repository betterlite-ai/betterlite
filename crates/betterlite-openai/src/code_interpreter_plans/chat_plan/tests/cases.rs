#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use serde_json::{Map, json};

#[cfg(test)]
use crate::code_execution_records::SANDBOX_KEY;
#[cfg(test)]
use crate::code_execution_records::tool_shapes::function_tool;
#[cfg(test)]
use crate::code_interpreter_plans::records::CodeInterpreterOutput;
#[cfg(test)]
use crate::sandbox_lifecycle::cache_operations::CacheEntry;

#[cfg(test)]
use super::{
    FixedClock, MockConfig, UnusedRegistry, UnusedToolRegistry, block_on, chat_tool_call,
    followup_values, metadata_value, plan_request, test_logger, test_map,
};

#[cfg(test)]
fn run_chat_plan(
    logger: &mut CodeInterpreterInterceptionLogger<MockConfig>,
    tool_calls: &[CodeExecutionToolCall],
    kwargs: &Map<String, Value>,
) -> AgenticLoopPlan {
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let request = plan_request(&messages, &optional_params, tool_calls, kwargs);
    let clock = FixedClock(Instant::now());
    block_on(build_chat_plan(
        logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap()
}

#[cfg(test)]
fn plan_with_one_chat_call() -> AgenticLoopPlan {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config);
    let tool_calls = [chat_tool_call(Some("call-1"), Some("call-1"), "print(1)")];
    run_chat_plan(
        &mut logger,
        &tool_calls,
        &test_map([(SANDBOX_KEY, json!("k"))]),
    )
}

#[cfg(test)]
fn tool_result(tool_call: &CodeExecutionToolCall) -> (ChatMessage, CodeInterpreterCall) {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let registry = UnusedRegistry;
    let tool_registry = UnusedToolRegistry;
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let container = json!({"id": "cnt-1"});
    block_on(build_chat_tool_result(
        &provider,
        None,
        &container,
        tool_call,
        Some("cnt-1".to_owned()),
    ))
    .unwrap()
}

#[cfg(test)]
#[test]
fn execution_runs_every_call_in_order_and_reuses_the_container() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config.clone());
    let tool_calls = [
        chat_tool_call(Some("call-1"), Some("call-1"), "print(1)"),
        chat_tool_call(Some("call-2"), Some("call-2"), "print(2)"),
    ];
    let kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let first = run_chat_plan(&mut logger, &tool_calls, &kwargs);
    run_chat_plan(&mut logger, &tool_calls, &kwargs);
    assert_eq!(config.create_count(), 1);
    assert_eq!(
        config.run_codes(),
        vec!["print(1)", "print(2)", "print(1)", "print(2)"]
    );
    let messages = followup_values(&first.request_patch.unwrap().messages.unwrap());
    assert_eq!(messages.len(), 4);
}

#[cfg(test)]
#[test]
fn execution_prunes_the_cache_before_creating_a_container() {
    let config = MockConfig::succeeding(json!({"id": "new"}), "4");
    let mut logger = test_logger(config.clone());
    let base = Instant::now();
    logger.cache.entries.insert(
        "stale".to_owned(),
        CacheEntry {
            container: json!({"id": "stale"}),
            params: None,
            last_accessed: base,
            identity: None,
        },
    );
    let tool_calls = [chat_tool_call(Some("call-1"), Some("call-1"), "print(1)")];
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let kwargs = test_map([(SANDBOX_KEY, json!("fresh"))]);
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(base + Duration::from_secs(901));
    block_on(build_chat_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap();
    assert_eq!(config.deleted_containers(), vec![json!({"id": "stale"})]);
    assert!(logger.cache.entries.contains_key("fresh"));
}

#[cfg(test)]
#[test]
fn tool_result_builds_the_tool_message_and_the_call_record() {
    let (message, call) = tool_result(&chat_tool_call(Some("call-1"), Some("call-1"), "print(1)"));
    assert_eq!(message.role, Role::Tool);
    assert_eq!(message.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(message.content, Some(MessageContent::Text("4".to_owned())));
    assert_eq!(call.code, "print(1)");
    assert_eq!(call.container_id.as_deref(), Some("cnt-1"));
    assert_eq!(
        call.outputs,
        vec![CodeInterpreterOutput::Logs {
            logs: "4".to_owned()
        }]
    );
}

#[cfg(test)]
#[test]
fn tool_result_prefers_the_id_and_falls_back_to_the_call_id() {
    let preferred = chat_tool_call(Some("id-1"), Some("call-2"), "print(1)");
    let (message, _) = tool_result(&preferred);
    assert_eq!(message.tool_call_id.as_deref(), Some("id-1"));
    let fallback = chat_tool_call(None, Some("call-2"), "print(1)");
    let (message, _) = tool_result(&fallback);
    assert_eq!(message.tool_call_id.as_deref(), Some("call-2"));
}

#[cfg(test)]
#[test]
fn tool_result_mints_a_hex_identifier_when_both_identifiers_are_absent() {
    let (message, _) = tool_result(&chat_tool_call(None, None, "print(1)"));
    let identifier = message.tool_call_id.unwrap();
    assert_eq!(identifier.len(), 32);
    assert!(
        identifier
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
}

#[cfg(test)]
#[test]
fn message_format_puts_the_assistant_message_before_the_tool_results() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config);
    let tool_calls = [
        chat_tool_call(Some("call-1"), Some("call-1"), "print(1)"),
        chat_tool_call(Some("call-2"), Some("call-2"), "print(2)"),
    ];
    let plan = run_chat_plan(
        &mut logger,
        &tool_calls,
        &test_map([(SANDBOX_KEY, json!("k"))]),
    );
    let messages = followup_values(&plan.request_patch.unwrap().messages.unwrap());
    assert_eq!(messages[0], json!({"role": "user", "content": "hi"}));
    assert_eq!(messages[1]["role"], json!("assistant"));
    assert_eq!(messages[1]["tool_calls"][0]["id"], json!("call-1"));
    assert_eq!(messages[1]["tool_calls"][1]["id"], json!("call-2"));
    assert_eq!(
        messages[2],
        json!({"role": "tool", "content": "4", "tool_call_id": "call-1"})
    );
    assert_eq!(
        messages[3],
        json!({"role": "tool", "content": "4", "tool_call_id": "call-2"})
    );
}

#[cfg(test)]
#[test]
fn assembly_sets_the_response_format_and_the_chat_dialect_tools() {
    let plan = plan_with_one_chat_call();
    assert!(plan.run_agentic_loop);
    assert!(!plan.terminate);
    assert!(plan.response_override.is_none());
    assert_eq!(
        metadata_value(&plan.metadata, "response_format"),
        json!("openai")
    );
    assert_eq!(
        metadata_value(&plan.metadata, "tool_type"),
        json!("code_interpreter")
    );
    assert_eq!(metadata_value(&plan.metadata, "sandbox_key"), json!("k"));
    assert_eq!(
        metadata_value(&plan.metadata, "is_session_scoped"),
        json!(false)
    );
    let patch = plan.request_patch.unwrap();
    assert_eq!(patch.model.as_deref(), Some("openai/gpt-5"));
    assert_eq!(
        followup_values(&patch.tools.unwrap()),
        vec![serde_json::to_value(function_tool(CallDialect::ChatCompletion)).unwrap()]
    );
}
