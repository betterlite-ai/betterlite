use betterlite_config::sections::websearch_request_hooks::Settings;
use betterlite_prompts::arguments::json::Json;
use serde_json::Number;

use crate::websearch_request_hooks::WebSearchInterceptionLogger;
use crate::websearch_request_hooks::config::{LoopValue, WebSearchInterceptionParams};
use crate::websearch_request_hooks::tests::{logger as bedrock_logger, object};

fn params(
    enabled_providers: Option<Vec<String>>,
    max_agentic_loops: Option<LoopValue>,
) -> WebSearchInterceptionParams {
    WebSearchInterceptionParams {
        enabled_providers,
        search_tool_name: None,
        max_agentic_loops,
    }
}

#[test]
fn from_config_yaml_defaults_to_bedrock() {
    let logger = WebSearchInterceptionLogger::from_config_yaml(params(None, None)).unwrap();
    assert_eq!(logger.enabled_providers, Some(vec!["bedrock".to_owned()]));
    assert_eq!(logger.search_tool_name, None);
    assert_eq!(logger.max_agentic_loops, None);
}

#[test]
fn from_config_yaml_keeps_an_empty_provider_list() {
    let logger =
        WebSearchInterceptionLogger::from_config_yaml(params(Some(Vec::new()), None)).unwrap();
    assert_eq!(logger.enabled_providers, Some(Vec::new()));
}

#[test]
fn from_config_yaml_accepts_a_whole_number_ceiling() {
    let logger = WebSearchInterceptionLogger::from_config_yaml(params(
        Some(vec!["bedrock".to_owned()]),
        Some(LoopValue::Integer(5)),
    ))
    .unwrap();
    assert_eq!(logger.max_agentic_loops, Some(5));
}

#[test]
fn from_config_yaml_rejects_a_bool_ceiling() {
    let error = WebSearchInterceptionLogger::from_config_yaml(params(
        Some(vec!["bedrock".to_owned()]),
        Some(LoopValue::Boolean(true)),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("max_agentic_loops"), "{error}");
}

#[test]
fn from_config_yaml_rejects_a_ceiling_below_one() {
    let error = WebSearchInterceptionLogger::from_config_yaml(params(
        Some(vec!["bedrock".to_owned()]),
        Some(LoopValue::Text("0".to_owned())),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("max_agentic_loops"), "{error}");
}

#[test]
fn initialize_from_proxy_config_prefers_litellm_settings() {
    let litellm = WebSearchInterceptionParams {
        enabled_providers: Some(vec!["openai".to_owned()]),
        search_tool_name: Some("settings".to_owned()),
        max_agentic_loops: None,
    };
    let callback = WebSearchInterceptionParams {
        enabled_providers: Some(vec!["bedrock".to_owned()]),
        search_tool_name: Some("callback".to_owned()),
        max_agentic_loops: None,
    };
    let logger =
        WebSearchInterceptionLogger::initialize_from_proxy_config(Some(litellm), Some(callback))
            .unwrap();
    assert_eq!(logger.enabled_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(logger.search_tool_name.as_deref(), Some("settings"));
}

#[test]
fn initialize_from_proxy_config_falls_back_to_callback_params() {
    let callback = WebSearchInterceptionParams {
        enabled_providers: Some(vec!["openai".to_owned()]),
        search_tool_name: Some("callback".to_owned()),
        max_agentic_loops: None,
    };
    let logger =
        WebSearchInterceptionLogger::initialize_from_proxy_config(None, Some(callback)).unwrap();
    assert_eq!(logger.search_tool_name.as_deref(), Some("callback"));
}

#[test]
fn initialize_from_proxy_config_defaults_when_both_are_missing() {
    let logger = WebSearchInterceptionLogger::initialize_from_proxy_config(None, None).unwrap();
    assert_eq!(logger.enabled_providers, Some(vec!["bedrock".to_owned()]));
}

#[test]
fn from_settings_maps_the_config_section() {
    let logger = WebSearchInterceptionLogger::from(Settings {
        enabled_providers: vec!["openai".to_owned()],
        search_tool_name: Some("section-tool".to_owned()),
        max_agentic_loops: Some(2),
    });
    assert_eq!(logger.enabled_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(logger.search_tool_name.as_deref(), Some("section-tool"));
    assert_eq!(logger.max_agentic_loops, Some(2));
}

#[test]
fn the_pre_request_hook_inserts_the_instance_ceiling() {
    let mut logger = bedrock_logger(&["bedrock"]);
    logger.max_agentic_loops = Some(5);
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger.async_pre_request_hook(&request).unwrap().unwrap();
    assert_eq!(
        converted.get("max_agentic_loops"),
        Some(&Json::Number(Number::from(5)))
    );
}

#[test]
fn the_pre_request_hook_fills_a_null_ceiling() {
    let mut logger = bedrock_logger(&["bedrock"]);
    logger.max_agentic_loops = Some(5);
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"max_agentic_loops":null,"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger.async_pre_request_hook(&request).unwrap().unwrap();
    assert_eq!(
        converted.get("max_agentic_loops"),
        Some(&Json::Number(Number::from(5)))
    );
}

#[test]
fn the_pre_request_hook_keeps_the_callers_ceiling() {
    let mut logger = bedrock_logger(&["bedrock"]);
    logger.max_agentic_loops = Some(5);
    let request = object(
        r#"{"litellm_params":{"custom_llm_provider":"bedrock"},"max_agentic_loops":2,"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger.async_pre_request_hook(&request).unwrap().unwrap();
    assert_eq!(
        converted.get("max_agentic_loops"),
        Some(&Json::Number(Number::from(2)))
    );
}

#[test]
fn the_deployment_hook_leaves_the_ceiling_alone() {
    let mut logger = bedrock_logger(&["bedrock"]);
    logger.max_agentic_loops = Some(5);
    let request = object(
        r#"{"custom_llm_provider":"bedrock","tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    );
    let converted = logger
        .async_pre_call_deployment_hook(&request, None)
        .unwrap()
        .unwrap();
    assert_eq!(converted.get("max_agentic_loops"), None);
}
