use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::Json;

use crate::websearch_request_hooks::EMIT_NATIVE_BLOCKS_KEY;
use crate::websearch_request_hooks::tests::{logger, object};

#[test]
fn the_deployment_hook_sets_the_native_blocks_flag() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        converted.get(EMIT_NATIVE_BLOCKS_KEY),
        Some(&Json::Bool(true))
    );
    assert_eq!(request.get(EMIT_NATIVE_BLOCKS_KEY), None);
}

#[test]
fn the_pre_request_hook_sets_the_native_blocks_flag() {
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_request_hook(&request)
        .unwrap()
        .unwrap();
    assert_eq!(
        converted.get(EMIT_NATIVE_BLOCKS_KEY),
        Some(&Json::Bool(true))
    );
    assert_eq!(request.get(EMIT_NATIVE_BLOCKS_KEY), None);
}

#[test]
fn the_deployment_hook_leaves_the_flag_unset_without_a_native_tool() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"function","function":{"name":"litellm_web_search"}}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get(EMIT_NATIVE_BLOCKS_KEY), None);
}

#[test]
fn the_pre_request_hook_leaves_the_flag_unset_without_a_native_tool() {
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"tools":[{"type":"custom","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_request_hook(&request)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get(EMIT_NATIVE_BLOCKS_KEY), None);
}

#[test]
fn the_responses_path_leaves_the_flag_unset() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap()
        .unwrap();
    assert_eq!(converted.get(EMIT_NATIVE_BLOCKS_KEY), None);
}
