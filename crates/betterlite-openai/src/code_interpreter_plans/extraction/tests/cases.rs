#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use betterlite_context::agentic_loop_plan::AgenticLoopPlan;

#[cfg(test)]
use crate::code_execution_records::SANDBOX_KEY;
#[cfg(test)]
use crate::code_execution_records::container_extraction;
#[cfg(test)]
use crate::code_interpreter_plans::initialization::InterceptionLogger;
#[cfg(test)]
use crate::code_interpreter_plans::responses_plan::build_responses_plan;

#[cfg(test)]
use super::{
    FixedClock, MockConfig, UnusedRegistry, UnusedToolRegistry, block_on, execution_tool_call,
    metadata_value, plan_request, test_logger, test_map, test_object,
};

#[cfg(test)]
fn run_responses_plan(
    logger: &mut InterceptionLogger<MockConfig>,
    kwargs: &Map<String, Value>,
) -> AgenticLoopPlan {
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let request = plan_request(&messages, &optional_params, &tool_calls, kwargs);
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
fn sample_logger() -> InterceptionLogger<MockConfig> {
    test_logger(MockConfig::succeeding(
        test_object([("id", json!("cnt-1"))]),
        "4",
    ))
}

#[cfg(test)]
#[test]
fn sandbox_key_from_the_request_reaches_the_plan_metadata() {
    let kwargs = test_map([(SANDBOX_KEY, json!("request-key"))]);
    let plan = run_responses_plan(&mut sample_logger(), &kwargs);
    assert_eq!(
        metadata_value(&plan.metadata, "sandbox_key"),
        json!("request-key")
    );
}

#[cfg(test)]
#[test]
fn sandbox_key_is_empty_in_the_metadata_without_a_request_key() {
    let plan = run_responses_plan(&mut sample_logger(), &Map::new());
    assert_eq!(metadata_value(&plan.metadata, "sandbox_key"), json!(""));
}

#[cfg(test)]
#[test]
fn sandbox_key_is_read_only_from_a_string_marker() {
    assert_eq!(
        container_extraction::sandbox_key(&test_map([(SANDBOX_KEY, json!("k"))])).as_deref(),
        Some("k")
    );
    assert!(container_extraction::sandbox_key(&test_map([(SANDBOX_KEY, json!(7_i32))])).is_none());
    assert!(container_extraction::sandbox_key(&Map::new()).is_none());
}

#[cfg(test)]
#[test]
fn session_scope_is_true_only_for_the_boolean_marker() {
    let user_key = json!("user-key");
    assert_eq!(
        container_identity(&test_map([
            (SESSION_SCOPED_KEY, json!(true)),
            ("user_api_key_hash", user_key.clone()),
        ]))
        .as_deref(),
        Some("user-key")
    );
    assert!(
        container_identity(&test_map([
            (SESSION_SCOPED_KEY, json!(false)),
            ("user_api_key_hash", user_key.clone()),
        ]))
        .is_none()
    );
    assert!(
        container_identity(&test_map([
            (SESSION_SCOPED_KEY, json!("true")),
            ("user_api_key_hash", user_key),
        ]))
        .is_none()
    );
    assert!(container_identity(&Map::new()).is_none());
}

#[cfg(test)]
#[test]
fn session_scope_metadata_records_the_flag_for_a_session_scoped_plan() {
    let kwargs = test_map([
        ("user_api_key_hash", json!("user-key")),
        (SESSION_SCOPED_KEY, json!(true)),
        (SANDBOX_KEY, json!("user-key:sess-1")),
    ]);
    let plan = run_responses_plan(&mut sample_logger(), &kwargs);
    assert_eq!(
        metadata_value(&plan.metadata, "is_session_scoped"),
        json!(true)
    );
}

#[cfg(test)]
#[test]
fn session_scope_metadata_records_false_for_a_request_scoped_plan() {
    let plan = run_responses_plan(&mut sample_logger(), &test_map([(SANDBOX_KEY, json!("k"))]));
    assert_eq!(
        metadata_value(&plan.metadata, "is_session_scoped"),
        json!(false)
    );
}

#[cfg(test)]
#[test]
fn identity_is_the_user_key_hash_only_for_session_scoped_calls() {
    let scoped = test_map([
        (SESSION_SCOPED_KEY, json!(true)),
        ("user_api_key_hash", json!("user-key")),
    ]);
    assert_eq!(container_identity(&scoped).as_deref(), Some("user-key"));
    assert_eq!(
        container_identity(&test_map([(SESSION_SCOPED_KEY, json!(true))])).as_deref(),
        Some("")
    );
    assert!(container_identity(&test_map([("user_api_key_hash", json!("user-key"))])).is_none());
}

#[cfg(test)]
#[test]
fn identity_reaches_the_cache_entry() {
    let kwargs = test_map([
        ("user_api_key_hash", json!("user-key")),
        (SESSION_SCOPED_KEY, json!(true)),
        (SANDBOX_KEY, json!("user-key:sess-1")),
    ]);
    let mut logger = sample_logger();
    run_responses_plan(&mut logger, &kwargs);
    let entry = &logger.cache.entries["user-key:sess-1"];
    assert_eq!(entry.identity.as_deref(), Some("user-key"));
}
