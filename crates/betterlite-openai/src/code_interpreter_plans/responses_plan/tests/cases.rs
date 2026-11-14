#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::code_execution_records::SANDBOX_KEY;
#[cfg(test)]
use crate::code_execution_records::tool_shapes::function_tool;
#[cfg(test)]
use crate::sandbox_lifecycle::cache_operations::CacheEntry;

#[cfg(test)]
use super::{
    FixedClock, MockConfig, UnusedRegistry, UnusedToolRegistry, block_on, chat_tool_call,
    execution_tool_call, followup_values, metadata_value, plan_request, test_logger, test_map,
};

#[cfg(test)]
fn run_plan(
    logger: &mut CodeInterpreterInterceptionLogger<MockConfig>,
    optional_params: &Map<String, Value>,
    tool_calls: &[CodeExecutionToolCall],
    kwargs: &Map<String, Value>,
) -> AgenticLoopPlan {
    let messages = json!([{"role": "user", "content": "hi"}]);
    let request = plan_request(&messages, optional_params, tool_calls, kwargs);
    let clock = FixedClock(Instant::now());
    block_on(build_responses_plan(
        logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap()
}

#[cfg(test)]
fn plan_with_one_call(code: &str, stdout: &str) -> AgenticLoopPlan {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), stdout);
    let mut logger = test_logger(config);
    let tool_calls = [execution_tool_call(Some("call-1"), code)];
    run_plan(
        &mut logger,
        &Map::new(),
        &tool_calls,
        &test_map([(SANDBOX_KEY, json!("k"))]),
    )
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
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let kwargs = test_map([(SANDBOX_KEY, json!("fresh"))]);
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(base + Duration::from_secs(901));
    block_on(build_responses_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap();
    assert_eq!(config.deleted_containers(), vec![json!({"id": "stale"})]);
    assert!(logger.cache.entries.contains_key("fresh"));
    assert!(!logger.cache.entries.contains_key("stale"));
}

#[cfg(test)]
#[test]
fn execution_runs_every_call_in_order_and_reuses_the_container() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config.clone());
    let tool_calls = [
        execution_tool_call(Some("call-1"), "print(1)"),
        execution_tool_call(Some("call-2"), "print(2)"),
    ];
    let kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let first = run_plan(&mut logger, &Map::new(), &tool_calls, &kwargs);
    run_plan(&mut logger, &Map::new(), &tool_calls, &kwargs);
    assert_eq!(config.create_count(), 1);
    assert_eq!(
        config.run_codes(),
        vec!["print(1)", "print(2)", "print(1)", "print(2)"]
    );
    let messages = followup_values(&first.request_patch.unwrap().messages.unwrap());
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[1]["type"], json!("function_call"));
    assert_eq!(messages[2]["type"], json!("function_call_output"));
    assert_eq!(messages[3]["type"], json!("function_call"));
    assert_eq!(messages[4]["type"], json!("function_call_output"));
}

#[cfg(test)]
#[test]
fn call_records_serialize_the_code_interpreter_call_shape() {
    let plan = plan_with_one_call("print(1)", "4");
    let calls = metadata_value(&plan.metadata, "code_interpreter_calls");
    let call = calls.as_array().unwrap()[0].clone();
    let id = call["id"].as_str().unwrap();
    assert!(id.starts_with("ci_"));
    assert_eq!(id.len(), 35);
    assert!(
        id[3..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    let mut without_id = call.as_object().unwrap().clone();
    without_id.remove("id");
    assert_eq!(
        Value::Object(without_id),
        json!({
            "type": "code_interpreter_call",
            "status": "completed",
            "code": "print(1)",
            "container_id": "cnt-1",
            "outputs": [{"type": "logs", "logs": "4"}]
        })
    );
}

#[cfg(test)]
#[test]
fn call_records_omit_empty_logs_and_a_missing_container_id() {
    let config = MockConfig::succeeding(json!({"meta": "no-id"}), "");
    let mut logger = test_logger(config);
    let tool_calls = [execution_tool_call(Some("call-1"), "pass")];
    let plan = run_plan(
        &mut logger,
        &Map::new(),
        &tool_calls,
        &test_map([(SANDBOX_KEY, json!("k"))]),
    );
    let calls = metadata_value(&plan.metadata, "code_interpreter_calls");
    let call = calls.as_array().unwrap()[0].clone();
    assert_eq!(call["outputs"], json!([]));
    assert_eq!(call["container_id"], json!(null));
    assert_eq!(call["code"], json!("pass"));
}

#[cfg(test)]
#[test]
fn message_format_appends_a_function_call_and_output_per_execution() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config);
    let tool_calls = [
        execution_tool_call(Some("call-1"), "print(1)"),
        chat_tool_call(None, None, "print(2)"),
    ];
    let plan = run_plan(
        &mut logger,
        &Map::new(),
        &tool_calls,
        &test_map([(SANDBOX_KEY, json!("k"))]),
    );
    let messages = followup_values(&plan.request_patch.unwrap().messages.unwrap());
    assert_eq!(messages[0], json!({"role": "user", "content": "hi"}));
    assert_eq!(
        messages[1],
        json!({
            "type": "function_call",
            "call_id": "call-1",
            "name": "litellm_code_execution",
            "arguments": "{\"code\":\"print(1)\"}"
        })
    );
    assert_eq!(
        messages[2],
        json!({"type": "function_call_output", "call_id": "call-1", "output": "4"})
    );
    assert_eq!(messages[3]["call_id"], json!(null));
    assert_eq!(messages[4]["call_id"], json!(null));
}

#[cfg(test)]
#[test]
fn assembly_sets_the_plan_fields_and_metadata() {
    let plan = plan_with_one_call("print(1)", "4");
    assert!(plan.run_agentic_loop);
    assert!(!plan.terminate);
    assert!(plan.response_override.is_none());
    assert!(plan.stop_reason.is_none());
    assert_eq!(
        metadata_value(&plan.metadata, "tool_type"),
        json!("code_interpreter")
    );
    assert_eq!(metadata_value(&plan.metadata, "sandbox_key"), json!("k"));
    assert_eq!(
        metadata_value(&plan.metadata, "is_session_scoped"),
        json!(false)
    );
    assert!(plan.metadata.get("response_format").is_none());
    assert_eq!(
        metadata_value(&plan.metadata, "code_interpreter_calls")
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[cfg(test)]
#[test]
fn assembly_filters_the_follow_up_request() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config);
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let optional_params = test_map([
        ("tools", json!([{"type": "code_interpreter"}])),
        ("tool_choice", json!({"type": "code_interpreter"})),
        ("temperature", json!(0.2)),
    ]);
    let kwargs = test_map([
        ("model", json!("openai/gpt-5")),
        ("litellm_logging_obj", json!("obj")),
    ]);
    let plan = run_plan(&mut logger, &optional_params, &tool_calls, &kwargs);
    let patch = plan.request_patch.unwrap();
    assert_eq!(patch.model.as_deref(), Some("openai/gpt-5"));
    let tools = followup_values(&patch.tools.unwrap());
    assert_eq!(
        tools,
        vec![serde_json::to_value(function_tool(CallDialect::Responses)).unwrap()]
    );
    assert_eq!(
        metadata_value(&patch.optional_params, "temperature"),
        json!(0.2)
    );
    assert!(patch.optional_params.get("tools").is_none());
    assert!(patch.optional_params.get("tool_choice").is_none());
    assert_eq!(
        metadata_value(&patch.kwargs, "model"),
        json!("openai/gpt-5")
    );
    assert!(patch.kwargs.get("litellm_logging_obj").is_none());
}
