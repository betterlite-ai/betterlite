use crate::tools::openai_web_search_tool;
use crate::websearch_request_hooks::tests::{list, logger, object, tool_name};
use crate::websearch_request_hooks::tool_rewrite::encode;

#[test]
fn converts_a_native_anthropic_tool_on_a_chat_completion_call() {
    let request = object(
        r#"{"model":"bedrock/claude","custom_llm_provider":"bedrock","api_key":"k","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    let expected = encode(&openai_web_search_tool()).unwrap();
    assert_eq!(list(&converted, "tools").len(), 1);
    assert_eq!(list(&converted, "tools")[0], expected);
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
    assert_eq!(converted.get("api_key"), request.get("api_key"));
    assert_eq!(converted.get("model"), request.get("model"));
}

#[test]
fn converts_only_the_web_search_tool_and_keeps_the_others() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"},{"type":"function","function":{"name":"calculator","parameters":{"type":"object"}}}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(list(&converted, "tools")[1], list(&request, "tools")[1]);
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_the_legacy_websearch_marker_but_not_a_client_tool_with_a_schema() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"name":"WebSearch"},{"name":"WebSearch","input_schema":{"type":"object","properties":{"query":{"type":"string"}}}}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
    assert_eq!(list(&converted, "tools")[1], list(&request, "tools")[1]);
}

#[test]
fn converts_claude_codes_web_search_tool_with_a_type() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"custom","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_the_litellm_function_wrapper_too() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"function","function":{"name":"litellm_web_search"}}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    let expected = encode(&openai_web_search_tool()).unwrap();
    assert_eq!(list(&converted, "tools")[0], expected);
}

#[test]
fn converts_a_native_tool_when_the_provider_comes_from_the_model_name() {
    let request = object(
        r#"{"model":"bedrock/us.anthropic.claude","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_a_native_tool_when_the_provider_is_nested_in_litellm_params() {
    let request = object(
        r#"{"model":"claude","litellm_params":{"custom_llm_provider":"bedrock"},"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_the_same_tool_on_an_async_chat_call() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(
            &request,
            Some(betterlite_core::routes::CallType::Acompletion),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}
