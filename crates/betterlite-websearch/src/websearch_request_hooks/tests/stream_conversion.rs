use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::websearch_request_hooks::tests::{logger, object};
use crate::websearch_request_hooks::{CONVERTED_STREAM_KEY, WebSearchError};

fn chat_request() -> betterlite_prompts::arguments::json::Object {
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
    assert_eq!(converted.get(CONVERTED_STREAM_KEY), Some(&Json::Bool(true)));
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

#[rstest]
#[case::false_value("false")]
#[case::null("null")]
fn a_non_streaming_request_keeps_its_stream_and_gains_no_flag(#[case] value: &str) {
    let source = format!(
        r#"{{"custom_llm_provider":"bedrock","stream":{value},"tools":[{{"type":"web_search_20250305","name":"web_search"}}]}}"#
    );
    let converted = logger(&["bedrock"])
        .async_pre_call_deployment_hook(&object(&source), None)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get(CONVERTED_STREAM_KEY), None);
    assert_ne!(converted.get("stream"), Some(&Json::Bool(true)));
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
