#[cfg(test)]
use betterlite_models::catalog::BundledCatalog;
#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::code_execution_records::tool_shapes::function_tool;

#[cfg(test)]
use super::{empty_logger, test_map};

#[cfg(test)]
fn hook<C>(
    logger: &InterceptionLogger<C>,
    kwargs: &mut Map<String, Value>,
    call_type: Option<CallType>,
) -> bool {
    pre_call_deployment_hook(logger, kwargs, call_type).unwrap()
}

#[cfg(test)]
fn code_interpreter_tools() -> Value {
    json!([{"type": "code_interpreter"}])
}

#[cfg(test)]
#[test]
fn root_metadata_strips_interception_markers_from_a_root_request() {
    let mut logger = empty_logger::<()>();
    logger.enabled = false;
    let mut kwargs = test_map([
        (INTERCEPTION_ACTIVE_KEY, json!(true)),
        (SANDBOX_KEY, json!("old-key")),
        (
            "litellm_metadata",
            json!({
                "_code_interpreter_interception_active": true,
                "_websearch_interception_active": true,
                "_agentic_loop_depth": 1,
                "max_agentic_loops": 3,
                "session_id": "sess-1"
            }),
        ),
    ]);
    assert!(!hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert!(kwargs.get(INTERCEPTION_ACTIVE_KEY).is_none());
    assert!(kwargs.get(SANDBOX_KEY).is_none());
    assert_eq!(
        kwargs.get("litellm_metadata"),
        Some(&json!({"session_id": "sess-1"}))
    );
}

#[cfg(test)]
#[test]
fn root_metadata_keeps_the_markers_during_a_follow_up_iteration() {
    let mut logger = empty_logger::<()>();
    logger.enabled = false;
    let mut kwargs = test_map([
        (INTERCEPTION_ACTIVE_KEY, json!(true)),
        (SANDBOX_KEY, json!("kept")),
        ("_agentic_loop_depth", json!(1)),
        (
            "litellm_metadata",
            json!({"_websearch_interception_active": true}),
        ),
    ]);
    let before = Value::Object(kwargs.clone());
    assert!(!hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert_eq!(Value::Object(kwargs), before);
}

#[cfg(test)]
#[rstest]
#[case::null(json!(null))]
#[case::zero(json!(0))]
#[case::boolean_false(json!(false))]
fn root_metadata_treats_a_missing_zero_or_empty_depth_as_the_root(#[case] depth: Value) {
    let mut logger = empty_logger::<()>();
    logger.enabled = false;
    let mut kwargs = test_map([
        (INTERCEPTION_ACTIVE_KEY, json!(true)),
        ("_agentic_loop_depth", depth),
    ]);
    assert!(!hook(&logger, &mut kwargs, None));
    assert!(kwargs.get(INTERCEPTION_ACTIVE_KEY).is_none());
}

#[cfg(test)]
#[test]
fn root_metadata_returns_false_when_the_hook_does_not_apply() {
    let mut logger = empty_logger::<()>();
    logger.enabled = false;
    let mut disabled = test_map([("tools", code_interpreter_tools())]);
    assert!(!hook(&logger, &mut disabled, Some(CallType::Responses)));
    assert!(disabled.get(INTERCEPTION_ACTIVE_KEY).is_none());

    let logger = empty_logger::<()>();
    let mut no_call_type = test_map([("tools", code_interpreter_tools())]);
    assert!(!hook(&logger, &mut no_call_type, None));
    assert!(no_call_type.get(INTERCEPTION_ACTIVE_KEY).is_none());

    let mut no_tools = Map::new();
    assert!(!hook(&logger, &mut no_tools, Some(CallType::Completion)));
    assert!(no_tools.get(INTERCEPTION_ACTIVE_KEY).is_none());

    let mut native_tools = test_map([(
        "tools",
        json!([{"type": "function", "name": "get_weather"}]),
    )]);
    assert!(!hook(&logger, &mut native_tools, Some(CallType::Responses)));
    assert!(native_tools.get(INTERCEPTION_ACTIVE_KEY).is_none());
}

#[cfg(test)]
#[test]
fn root_metadata_returns_false_when_the_provider_is_not_enabled() {
    let mut logger = empty_logger::<()>();
    logger.enabled_providers = Some(vec!["azure".to_owned()]);
    let mut blocked = test_map([
        ("model", json!("openai/gpt-5")),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(!hook(&logger, &mut blocked, Some(CallType::Responses)));
    assert!(blocked.get(INTERCEPTION_ACTIVE_KEY).is_none());

    let mut allowed = test_map([
        ("model", json!("openai/gpt-5")),
        ("custom_llm_provider", json!("azure")),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut allowed, Some(CallType::Responses)));
    assert_eq!(allowed.get(INTERCEPTION_ACTIVE_KEY), Some(&json!(true)));

    let mut bundled = empty_logger::<()>();
    bundled.enabled_providers = Some(vec!["openai".to_owned()]);
    bundled.catalog = BundledCatalog::default().get();
    let mut bare = test_map([
        ("model", json!("gpt-4o")),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&bundled, &mut bare, Some(CallType::Responses)));
    assert_eq!(bare.get(INTERCEPTION_ACTIVE_KEY), Some(&json!(true)));
}

#[cfg(test)]
#[test]
fn sandbox_key_combines_identity_and_session_id() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("user_api_key_hash", json!("user-1")),
        ("metadata", json!({"session_id": "sess-1"})),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert_eq!(kwargs.get(INTERCEPTION_ACTIVE_KEY), Some(&json!(true)));
    assert_eq!(kwargs.get(SANDBOX_KEY), Some(&json!("user-1:sess-1")));
    assert_eq!(kwargs.get(SESSION_SCOPED_KEY), Some(&json!(true)));
    let metadata = kwargs.get("litellm_metadata").unwrap().as_object().unwrap();
    assert_eq!(metadata.get(SANDBOX_KEY), Some(&json!("user-1:sess-1")));
    assert_eq!(metadata.get(SESSION_SCOPED_KEY), Some(&json!(true)));
    assert_eq!(metadata.get(INTERCEPTION_ACTIVE_KEY), Some(&json!(true)));
}

#[cfg(test)]
#[test]
fn sandbox_key_is_the_session_id_alone_when_the_identity_is_empty() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("litellm_metadata", json!({"session_id": "sess-1"})),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert_eq!(kwargs.get(SANDBOX_KEY), Some(&json!("sess-1")));
}

#[cfg(test)]
#[test]
fn sandbox_key_is_a_fresh_uuid_for_a_request_scoped_call() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([("tools", code_interpreter_tools())]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    let key = kwargs.get(SANDBOX_KEY).and_then(Value::as_str).unwrap();
    assert_eq!(key.len(), 32);
    assert!(key.chars().all(|character| character.is_ascii_hexdigit()));
    assert!(kwargs.get(SESSION_SCOPED_KEY).is_none());
}

#[cfg(test)]
#[test]
fn sandbox_key_is_a_fresh_uuid_when_the_metadata_session_id_is_empty() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("user_api_key_hash", json!("user-1")),
        ("metadata", json!({"session_id": ""})),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    let key = kwargs.get(SANDBOX_KEY).and_then(Value::as_str).unwrap();
    assert_eq!(key.len(), 32);
    assert!(key.chars().all(|character| character.is_ascii_hexdigit()));
    assert!(kwargs.get(SESSION_SCOPED_KEY).is_none());
}

#[cfg(test)]
#[test]
fn sandbox_key_falls_through_to_the_litellm_metadata_session_id() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("user_api_key_hash", json!("user-1")),
        ("metadata", json!({"session_id": ""})),
        ("litellm_metadata", json!({"session_id": "sess-1"})),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert_eq!(kwargs.get(SANDBOX_KEY), Some(&json!("user-1:sess-1")));
    assert_eq!(kwargs.get(SESSION_SCOPED_KEY), Some(&json!(true)));
}

#[cfg(test)]
#[test]
fn stream_conversion_turns_a_streaming_call_into_a_buffered_one() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([("stream", json!(true)), ("tools", code_interpreter_tools())]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    assert_eq!(kwargs.get("stream"), Some(&json!(false)));
    assert_eq!(kwargs.get(CONVERTED_STREAM_KEY), Some(&json!(true)));
    let metadata = kwargs.get("litellm_metadata").unwrap().as_object().unwrap();
    assert_eq!(metadata.get(CONVERTED_STREAM_KEY), Some(&json!(true)));
}

#[cfg(test)]
#[test]
fn stream_conversion_leaves_a_non_streaming_call_alone() {
    let logger = empty_logger::<()>();
    let mut explicit = test_map([
        ("stream", json!(false)),
        ("tools", code_interpreter_tools()),
    ]);
    assert!(hook(&logger, &mut explicit, Some(CallType::Responses)));
    assert_eq!(explicit.get("stream"), Some(&json!(false)));
    assert!(explicit.get(CONVERTED_STREAM_KEY).is_none());

    let mut absent = test_map([("tools", code_interpreter_tools())]);
    assert!(hook(&logger, &mut absent, Some(CallType::Responses)));
    assert!(absent.get("stream").is_none());
    assert!(absent.get(CONVERTED_STREAM_KEY).is_none());
}

#[cfg(test)]
#[test]
fn tool_rewriting_replaces_code_interpreter_tools_and_keeps_the_rest_in_order() {
    let logger = empty_logger::<()>();
    let weather = json!({"type": "function", "name": "get_weather"});
    let mut kwargs = test_map([(
        "tools",
        json!([{"type": "code_interpreter"}, weather.clone(), {"type": "code_interpreter"}]),
    )]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Responses)));
    let replacement = serde_json::to_value(function_tool(CallDialect::Responses)).unwrap();
    let tools = kwargs.get("tools").unwrap().as_array().unwrap();
    assert_eq!(tools, &vec![replacement.clone(), weather, replacement]);
}

#[cfg(test)]
#[rstest]
#[case::completion(CallType::Completion)]
#[case::acompletion(CallType::Acompletion)]
fn tool_rewriting_uses_the_chat_completion_shape_for_a_completion_call(
    #[case] call_type: CallType,
) {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([("tools", code_interpreter_tools())]);
    assert!(hook(&logger, &mut kwargs, Some(call_type)));
    let replacement = serde_json::to_value(function_tool(CallDialect::ChatCompletion)).unwrap();
    assert_eq!(kwargs.get("tools"), Some(&json!([replacement])));
}

#[cfg(test)]
#[test]
fn tool_rewriting_uses_the_responses_shape_for_an_aresponses_call() {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("tools", code_interpreter_tools()),
        ("tool_choice", json!({"type": "code_interpreter"})),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(CallType::Aresponses)));
    let replacement = serde_json::to_value(function_tool(CallDialect::Responses)).unwrap();
    assert_eq!(kwargs.get("tools"), Some(&json!([replacement])));
    let choice = serde_json::to_value(function_tool_choice(CallDialect::Responses)).unwrap();
    assert_eq!(kwargs.get("tool_choice"), Some(&choice));
}

#[cfg(test)]
#[rstest]
#[case::responses(
    CallType::Responses,
    json!({"type": "code_interpreter"}),
    CallDialect::Responses
)]
#[case::completion(
    CallType::Completion,
    json!({"type": "function", "name": "litellm_code_execution"}),
    CallDialect::ChatCompletion
)]
#[case::acompletion(
    CallType::Acompletion,
    json!({"type": "function", "name": "litellm_code_execution"}),
    CallDialect::ChatCompletion
)]
fn tool_choice_targeting_code_interpreter_is_rewritten_to_the_function_choice(
    #[case] call_type: CallType,
    #[case] requested: Value,
    #[case] dialect: CallDialect,
) {
    let logger = empty_logger::<()>();
    let mut kwargs = test_map([
        ("tools", code_interpreter_tools()),
        ("tool_choice", requested),
    ]);
    assert!(hook(&logger, &mut kwargs, Some(call_type)));
    let choice = serde_json::to_value(function_tool_choice(dialect)).unwrap();
    assert_eq!(kwargs.get("tool_choice"), Some(&choice));
}

#[cfg(test)]
#[test]
fn tool_choice_for_another_function_or_a_plain_value_is_left_unchanged() {
    let logger = empty_logger::<()>();
    let weather_choice = json!({"type": "function", "function": {"name": "get_weather"}});
    let mut named = test_map([
        ("tools", code_interpreter_tools()),
        ("tool_choice", weather_choice.clone()),
    ]);
    assert!(hook(&logger, &mut named, Some(CallType::Responses)));
    assert_eq!(named.get("tool_choice"), Some(&weather_choice));

    let mut automatic = test_map([
        ("tools", code_interpreter_tools()),
        ("tool_choice", json!("auto")),
    ]);
    assert!(hook(&logger, &mut automatic, Some(CallType::Responses)));
    assert_eq!(automatic.get("tool_choice"), Some(&json!("auto")));
}
