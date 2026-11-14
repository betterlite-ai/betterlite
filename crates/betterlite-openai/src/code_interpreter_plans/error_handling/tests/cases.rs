#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use serde_json::{Map, json};

#[cfg(test)]
use crate::code_execution_records::SANDBOX_KEY;
#[cfg(test)]
use crate::code_interpreter_plans::chat_plan::build_chat_plan;
#[cfg(test)]
use crate::code_interpreter_plans::responses_plan::build_responses_plan;

#[cfg(test)]
use super::{
    FixedClock, MockConfig, UnusedRegistry, UnusedToolRegistry, block_on, chat_tool_call,
    execution_tool_call, plan_request, test_logger, test_map,
};

#[cfg(test)]
#[test]
fn cleanup_deletes_the_container_when_a_responses_run_fails() {
    let config = MockConfig::failing_run(json!({"id": "cnt-1"}));
    let mut logger = test_logger(config.clone());
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(Instant::now());
    let failure = block_on(build_responses_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap_err();
    assert!(matches!(failure, PlanError::ToolExecution(_)));
    assert_eq!(config.deleted_containers(), vec![json!({"id": "cnt-1"})]);
    assert!(logger.cache.entries.is_empty());
}

#[cfg(test)]
#[test]
fn cleanup_deletes_the_container_when_a_chat_run_fails() {
    let config = MockConfig::failing_run(json!({"id": "cnt-1"}));
    let mut logger = test_logger(config.clone());
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [chat_tool_call(Some("call-1"), Some("call-1"), "print(1)")];
    let kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(Instant::now());
    let failure = block_on(build_chat_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap_err();
    assert!(matches!(failure, PlanError::ToolExecution(_)));
    assert_eq!(config.deleted_containers(), vec![json!({"id": "cnt-1"})]);
    assert!(logger.cache.entries.is_empty());
}

#[cfg(test)]
#[test]
fn cleanup_keeps_the_container_after_a_successful_plan() {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config.clone());
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(Instant::now());
    block_on(build_responses_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap();
    assert!(config.deleted_containers().is_empty());
    assert!(logger.cache.entries.contains_key("k"));
}

#[cfg(test)]
#[test]
fn cleanup_has_nothing_to_delete_without_a_sandbox_key() {
    let config = MockConfig::failing_run(json!({"id": "cnt-1"}));
    let mut logger = test_logger(config.clone());
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let kwargs = Map::new();
    let request = plan_request(&messages, &optional_params, &tool_calls, &kwargs);
    let clock = FixedClock(Instant::now());
    let failure = block_on(build_responses_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap_err();
    assert!(matches!(failure, PlanError::ToolExecution(_)));
    assert!(config.deleted_containers().is_empty());
    assert!(logger.cache.entries.is_empty());
}
