use serde_json::json;

use super::super::o_series;
use super::{
    Family, Fixture, Resolver, global_drop, map_probe, no_drop, object, request_drop, resolved_o3,
};

const O3_OPEN: &str = r#"{
    "o3": {
        "litellm_provider": "openai",
        "supports_function_calling": true,
        "supports_parallel_function_calling": true,
        "supports_response_schema": true,
        "supports_system_messages": true
    }
}"#;

pub(super) async fn o_series_capability_fallback() {
    let entries = r#"{
        "fixture-model": {
            "litellm_provider": "openai",
            "supports_function_calling": true,
            "supports_parallel_function_calling": true,
            "supports_response_schema": true
        }
    }"#;
    let mut fixture = Fixture::new(&[], entries);
    let catalog = fixture.catalog.clone();
    let supported =
        o_series::supported_params(&mut fixture, &mut Resolver(None), "fixture-model", &catalog)
            .await;
    for present in [
        "tools",
        "tool_choice",
        "function_call",
        "functions",
        "parallel_tool_calls",
        "response_format",
        "reasoning_effort",
    ] {
        assert!(supported.contains(&present), "{present} should be allowed");
    }
    for absent in [
        "logprobs",
        "top_p",
        "presence_penalty",
        "frequency_penalty",
        "top_logprobs",
    ] {
        assert!(!supported.contains(&absent), "{absent} should be omitted");
    }
    assert_eq!(
        supported
            .iter()
            .filter(|param| **param == "reasoning_effort")
            .count(),
        1
    );

    let entries = r#"{
        "fixture-model": {
            "litellm_provider": "openai",
            "supports_function_calling": false,
            "supports_parallel_function_calling": true,
            "supports_response_schema": true
        }
    }"#;
    let mut fixture = Fixture::new(&[], entries);
    let catalog = fixture.catalog.clone();
    let supported =
        o_series::supported_params(&mut fixture, &mut Resolver(None), "fixture-model", &catalog)
            .await;
    for absent in ["tools", "tool_choice", "function_call", "functions"] {
        assert!(!supported.contains(&absent), "{absent} should be omitted");
    }
    assert!(supported.contains(&"parallel_tool_calls"));
    assert!(supported.contains(&"response_format"));

    let entries = r#"{
        "fixture-model": {
            "litellm_provider": "openai",
            "supports_function_calling": true,
            "supports_parallel_function_calling": false,
            "supports_response_schema": true
        }
    }"#;
    let mut fixture = Fixture::new(&[], entries);
    let catalog = fixture.catalog.clone();
    let supported =
        o_series::supported_params(&mut fixture, &mut Resolver(None), "fixture-model", &catalog)
            .await;
    assert!(!supported.contains(&"parallel_tool_calls"));

    let entries = r#"{
        "fixture-model": {
            "litellm_provider": "openai",
            "supports_function_calling": true,
            "supports_parallel_function_calling": true,
            "supports_response_schema": false
        }
    }"#;
    let mut fixture = Fixture::new(&[], entries);
    let catalog = fixture.catalog.clone();
    let supported =
        o_series::supported_params(&mut fixture, &mut Resolver(None), "fixture-model", &catalog)
            .await;
    assert!(!supported.contains(&"response_format"));

    let mut fixture = Fixture::new(&["o3"], O3_OPEN);
    let catalog = fixture.catalog.clone();
    let supported = o_series::supported_params(
        &mut fixture,
        &mut Resolver(resolved_o3()),
        "openai/o3",
        &catalog,
    )
    .await;
    assert!(!supported.contains(&"tools"));
    assert!(!supported.contains(&"response_format"));
    assert!(!supported.contains(&"user"));
    assert!(supported.contains(&"reasoning_effort"));
}

pub(super) async fn o_series_temperature_limit() {
    let mut fixture = Fixture::new(&["o3"], O3_OPEN);
    let mut resolver = Resolver(resolved_o3());
    let mut non_default = object(json!({"max_tokens": 4, "temperature": null}));
    let mut optional = object(json!({}));
    map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        no_drop(),
        Family::OSeries,
    )
    .await
    .expect("a null temperature is omitted");
    assert_eq!(optional.get("max_completion_tokens"), Some(&json!(4)));
    assert!(!non_default.contains_key("max_tokens"));
    assert!(!optional.contains_key("temperature"));

    let mut non_default = object(json!({"temperature": 1, "max_tokens": 2}));
    let mut optional = object(json!({}));
    map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        no_drop(),
        Family::OSeries,
    )
    .await
    .expect("temperature 1 survives");
    assert_eq!(optional.get("temperature"), Some(&json!(1)));
    assert_eq!(optional.get("max_completion_tokens"), Some(&json!(2)));

    let mut non_default = object(json!({"temperature": 0.2}));
    let mut optional = object(json!({}));
    let failure = map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        no_drop(),
        Family::OSeries,
    )
    .await
    .expect_err("o-series models only accept temperature 1");
    assert_eq!(failure.status_code(), 400);

    let mut non_default = object(json!({"temperature": 0.2}));
    let mut optional = object(json!({"temperature": 0.2}));
    map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        request_drop(),
        Family::OSeries,
    )
    .await
    .expect("drop_params removes the temperature");
    assert!(!non_default.contains_key("temperature"));
    assert_eq!(optional.get("temperature"), Some(&json!(0.2)));

    let mut non_default = object(json!({"temperature": 0.2}));
    let mut optional = object(json!({}));
    map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        global_drop(),
        Family::OSeries,
    )
    .await
    .expect("the global drop removes the temperature");
    assert!(!non_default.contains_key("temperature"));

    let mut non_default = object(json!({
        "reasoning_effort": "high",
        "logprobs": true,
        "top_p": 0.5,
        "presence_penalty": 0.1,
        "frequency_penalty": 0.1,
        "top_logprobs": 2
    }));
    let mut optional = object(json!({}));
    map_probe(
        &mut fixture,
        &mut resolver,
        "o3",
        &mut non_default,
        &mut optional,
        no_drop(),
        Family::OSeries,
    )
    .await
    .expect("o-series sampling params are omitted, not rejected");
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("high")));
    for absent in [
        "logprobs",
        "top_p",
        "presence_penalty",
        "frequency_penalty",
        "top_logprobs",
    ] {
        assert!(!optional.contains_key(absent), "{absent} should be omitted");
    }
}
