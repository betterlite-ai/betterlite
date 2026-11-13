use crate::websearch_request_hooks::WebSearchError;
use crate::websearch_request_hooks::tests::{logger, object};
use crate::websearch_request_hooks::tool_rewrite::RewriteError;

fn native_request(provider: &str) -> betterlite_prompts::arguments::json::Object {
    object(&format!(
        r#"{{"custom_llm_provider":"{provider}","tools":[{{"type":"web_search_20250305","name":"web_search"}}]}}"#
    ))
}

fn litellm_request(provider: &str) -> betterlite_prompts::arguments::json::Object {
    object(&format!(
        r#"{{"litellm_params":{{"custom_llm_provider":"{provider}"}},"tools":[{{"type":"web_search_20250305","name":"web_search"}}]}}"#
    ))
}

#[test]
fn skips_a_provider_outside_the_enabled_list() {
    let enabled = logger(&["bedrock"]);
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&native_request("openai"), None)
            .unwrap(),
        None
    );
    assert_eq!(
        enabled
            .async_pre_request_hook(&litellm_request("openai"))
            .unwrap(),
        None
    );
}

#[test]
fn skips_every_provider_when_the_enabled_list_is_empty() {
    let enabled = logger(&[]);
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&native_request("bedrock"), None)
            .unwrap(),
        None
    );
    assert_eq!(
        enabled
            .async_pre_request_hook(&litellm_request("bedrock"))
            .unwrap(),
        None
    );
}

#[test]
fn a_missing_enabled_list_intercepts_any_provider() {
    let enabled = crate::websearch_request_hooks::WebSearchInterceptionLogger {
        enabled_providers: None,
        search_tool_name: None,
        max_agentic_loops: None,
    };
    assert!(
        enabled
            .async_pre_call_deployment_hook(&native_request("openai"), None)
            .unwrap()
            .is_some()
    );
    assert!(
        enabled
            .async_pre_request_hook(&litellm_request("openai"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn skips_a_request_without_tools() {
    let enabled = logger(&["bedrock"]);
    let request = object(r#"{"custom_llm_provider":"bedrock"}"#);
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&request, None)
            .unwrap(),
        None
    );
    let litellm = object(r#"{"litellm_params":{"custom_llm_provider":"bedrock"}}"#);
    assert_eq!(enabled.async_pre_request_hook(&litellm).unwrap(), None);
}

#[test]
fn skips_a_request_without_a_web_search_tool() {
    let enabled = logger(&["bedrock"]);
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"function","function":{"name":"calculator"}}]}"#,
    );
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&request, None)
            .unwrap(),
        None
    );
    let litellm = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"tools":[{"type":"function","function":{"name":"calculator"}}]}"#,
    );
    assert_eq!(enabled.async_pre_request_hook(&litellm).unwrap(), None);
}

#[test]
fn skips_a_request_with_empty_tools() {
    let enabled = logger(&["bedrock"]);
    let request = object(r#"{"custom_llm_provider":"bedrock","tools":[]}"#);
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&request, None)
            .unwrap(),
        None
    );
}

#[test]
fn skips_an_unknown_model_without_a_provider() {
    let enabled = logger(&["bedrock"]);
    let request = object(
        r#"{"model":"gpt-4o","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    assert_eq!(
        enabled
            .async_pre_call_deployment_hook(&request, None)
            .unwrap(),
        None
    );
}

#[test]
fn pre_request_skips_when_the_provider_is_missing_from_litellm_params() {
    let enabled = logger(&["bedrock"]);
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    assert_eq!(enabled.async_pre_request_hook(&request).unwrap(), None);
}

#[test]
fn a_non_list_tools_field_is_refused() {
    let enabled = logger(&["bedrock"]);
    let request = object(r#"{"custom_llm_provider":"bedrock","tools":"web_search"}"#);
    let error = enabled
        .async_pre_call_deployment_hook(&request, None)
        .unwrap_err();
    assert!(matches!(
        error,
        WebSearchError::Rewrite(RewriteError::Tools)
    ));
}

#[test]
fn a_non_object_litellm_params_is_refused() {
    let enabled = logger(&["bedrock"]);
    let request = object(
        r#"{"litellm_params":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    assert!(enabled.async_pre_request_hook(&request).is_err());
}

#[test]
fn a_non_string_provider_is_refused() {
    let enabled = logger(&["bedrock"]);
    let request = object(r#"{"custom_llm_provider":7}"#);
    assert!(
        enabled
            .async_pre_call_deployment_hook(&request, None)
            .is_err()
    );
}
