use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::websearch_request_hooks::tests::{json, logger, object};
use crate::websearch_request_hooks::{CONVERTED_STREAM_KEY, WebSearchError};

fn chat_request() -> Object {
    object(
        r#"{"custom_llm_provider":"bedrock","stream":true,"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    )
}

#[test]
fn the_deployment_hook_buffers_a_streamed_chat_call() {
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&chat_request(), None)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get("stream"), Some(&Json::Bool(false)));
    assert_eq!(
        converted.get("_websearch_interception_converted_stream"),
        Some(&Json::Bool(true))
    );
}

#[test]
fn the_pre_request_hook_buffers_a_streamed_call() {
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"stream":true,"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_request_hook(&request)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get("stream"), Some(&Json::Bool(false)));
    assert_eq!(converted.get(CONVERTED_STREAM_KEY), Some(&Json::Bool(true)));
}

#[test]
fn the_responses_path_buffers_a_stream_too() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","stream":true,"tools":[{"type":"web_search_preview"}]}"#,
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap()
        .unwrap();
    assert_eq!(converted.get("stream"), Some(&Json::Bool(false)));
    assert_eq!(converted.get(CONVERTED_STREAM_KEY), Some(&Json::Bool(true)));
}

#[derive(Clone, Copy, Debug)]
enum InterceptionHook {
    Deployment,
    PreRequest,
}

#[rstest]
#[case::deployment_false_value(InterceptionHook::Deployment, "false")]
#[case::deployment_null(InterceptionHook::Deployment, "null")]
#[case::pre_request_false_value(InterceptionHook::PreRequest, "false")]
#[case::pre_request_null(InterceptionHook::PreRequest, "null")]
fn either_hook_keeps_a_non_streaming_stream_and_adds_no_flag(
    #[case] hook: InterceptionHook,
    #[case] value: &str,
) {
    let source = format!(
        r#"{{"custom_llm_provider":"bedrock","litellm_params":{{"custom_llm_provider":"bedrock"}},"stream":{value},"tools":[{{"type":"web_search_20250305","name":"web_search"}}]}}"#
    );
    let request = object(&source);
    let converted = match hook {
        InterceptionHook::Deployment => {
            logger(&["bedrock"]).async_pre_call_deployment_hook(&request, None)
        }
        InterceptionHook::PreRequest => logger(&["bedrock"]).async_pre_request_hook(&request),
    }
    .unwrap()
    .unwrap();
    assert_eq!(converted.get("stream"), Some(&json(value)));
    assert_eq!(
        converted.get("_websearch_interception_converted_stream"),
        None
    );
}

#[test]
fn a_non_boolean_stream_is_refused() {
    let request = object(
        r#"{"custom_llm_provider":"bedrock","stream":"yes","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let error = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&request, None)
        .unwrap_err();
    assert!(matches!(error, WebSearchError::Stream));
}
