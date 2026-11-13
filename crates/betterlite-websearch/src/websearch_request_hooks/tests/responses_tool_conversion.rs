use betterlite_core::routes::CallType;

use crate::tools::responses_web_search_tool;
use crate::websearch_request_hooks::tests::{list, logger, object, tool_name};
use crate::websearch_request_hooks::tool_rewrite::encode;

#[test]
fn converts_a_responses_native_tool_to_the_flat_function() {
    let request =
        object(r#"{"custom_llm_provider":"openai","tools":[{"type":"web_search_preview"}]}"#);
    let converted = logger(&["openai"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap()
        .unwrap();
    let expected = encode(&responses_web_search_tool()).unwrap();
    assert_eq!(list(&converted, "tools").len(), 1);
    assert_eq!(list(&converted, "tools")[0], expected);
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_the_bare_web_search_type_on_the_async_responses_call() {
    let request = object(r#"{"custom_llm_provider":"openai","tools":[{"type":"web_search"}]}"#);
    let converted = logger(&["openai"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Aresponses))
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn converts_a_native_anthropic_type_on_the_responses_path() {
    let request = object(
        r#"{"custom_llm_provider":"openai","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger(&["openai"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap()
        .unwrap();
    assert_eq!(
        tool_name(&list(&converted, "tools")[0]),
        Some("litellm_web_search")
    );
}

#[test]
fn keeps_tools_that_are_not_responses_web_search_tools() {
    let request = object(
        r#"{"custom_llm_provider":"openai","tools":[{"type":"function","name":"get_weather"},{"type":"web_search_preview"}]}"#,
    );
    let converted = logger(&["openai"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap()
        .unwrap();
    assert_eq!(list(&converted, "tools")[0], list(&request, "tools")[0]);
    assert_eq!(
        tool_name(&list(&converted, "tools")[1]),
        Some("litellm_web_search")
    );
}

#[test]
fn leaves_the_request_alone_without_a_responses_web_search_tool() {
    let request = object(
        r#"{"custom_llm_provider":"openai","tools":[{"type":"function","name":"get_weather"}]}"#,
    );
    let outcome = logger(&["openai"])
        .async_pre_call_deployment_hook(&request, Some(CallType::Responses))
        .unwrap();
    assert_eq!(outcome, None);
}
