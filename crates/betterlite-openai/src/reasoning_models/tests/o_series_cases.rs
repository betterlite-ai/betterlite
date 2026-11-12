use serde_json::Value;

use super::super::{ReasoningError, o_series};
use super::{
    DropParams, Family, Fixture, Requests, Resolver, global_drop, map_probe, no_drop, request,
    request_drop, resolved_o3,
};
use crate::openai_base_config::Object;

const O3_OPEN: &str = r#"{
    "o3": {
        "litellm_provider": "openai",
        "supports_function_calling": true,
        "supports_parallel_function_calling": true,
        "supports_response_schema": true,
        "supports_system_messages": true
    }
}"#;

const FIXTURE_OPEN: &str = r#"{
    "fixture-capability-model": {
        "litellm_provider": "openai",
        "supports_function_calling": true,
        "supports_parallel_function_calling": true,
        "supports_response_schema": true
    }
}"#;

const FIXTURE_NO_FUNCTION: &str = r#"{
    "fixture-capability-model": {
        "litellm_provider": "openai",
        "supports_function_calling": false,
        "supports_parallel_function_calling": true,
        "supports_response_schema": true
    }
}"#;

const FIXTURE_NO_PARALLEL: &str = r#"{
    "fixture-capability-model": {
        "litellm_provider": "openai",
        "supports_function_calling": true,
        "supports_parallel_function_calling": false,
        "supports_response_schema": true
    }
}"#;

const FIXTURE_NO_SCHEMA: &str = r#"{
    "fixture-capability-model": {
        "litellm_provider": "openai",
        "supports_function_calling": true,
        "supports_parallel_function_calling": true,
        "supports_response_schema": false
    }
}"#;

async fn supported_for(fixture: &mut Fixture, model: &str) -> Vec<&'static str> {
    let catalog = fixture.catalog.clone();
    o_series::supported_params(fixture, &mut Resolver(None), model, &catalog).await
}

pub(super) async fn o_series_capability_fallback() {
    let mut open = Fixture::new(&[], FIXTURE_OPEN);
    let supported = supported_for(&mut open, "fixture-capability-model").await;
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
    fallback_gates().await;
    resolved_gate().await;
}

async fn fallback_gates() {
    let mut no_function = Fixture::new(&[], FIXTURE_NO_FUNCTION);
    let limited = supported_for(&mut no_function, "fixture-capability-model").await;
    for absent in ["tools", "tool_choice", "function_call", "functions"] {
        assert!(!limited.contains(&absent), "{absent} should be omitted");
    }
    assert!(limited.contains(&"parallel_tool_calls"));
    assert!(limited.contains(&"response_format"));

    let mut no_parallel = Fixture::new(&[], FIXTURE_NO_PARALLEL);
    let unpaired = supported_for(&mut no_parallel, "fixture-capability-model").await;
    assert!(!unpaired.contains(&"parallel_tool_calls"));

    let mut no_schema = Fixture::new(&[], FIXTURE_NO_SCHEMA);
    let schemaless = supported_for(&mut no_schema, "fixture-capability-model").await;
    assert!(!schemaless.contains(&"response_format"));
}

async fn resolved_gate() {
    let mut fixture = Fixture::new(&["o3"], O3_OPEN);
    let catalog = fixture.catalog.clone();
    let supported = o_series::supported_params(
        &mut fixture,
        &mut Resolver(Some(resolved_o3())),
        "openai/o3",
        &catalog,
    )
    .await;
    assert!(supported.contains(&"reasoning_effort"));
    assert!(!supported.contains(&"user"));
    assert!(!supported.contains(&"tools"));
    assert!(!supported.contains(&"response_format"));
}

async fn map_o3(
    fixture: &mut Fixture,
    model: &str,
    non_default: &mut Object,
    optional: &mut Object,
    drop_params: DropParams,
) -> Result<(), ReasoningError> {
    map_probe(
        fixture,
        &mut Resolver(Some(resolved_o3())),
        model,
        Requests {
            non_default,
            optional,
            drop_params,
        },
        Family::OSeries,
    )
    .await
}

pub(super) async fn o_series_temperature_limit() {
    let mut fixture = Fixture::new(&["o3"], O3_OPEN);
    temperature_mapping(&mut fixture).await;
    temperature_rejection(&mut fixture).await;
    sampling_omitted(&mut fixture).await;
}

async fn temperature_mapping(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"max_tokens": 4, "temperature": null}"#);
    let mut optional = request("{}");
    map_o3(fixture, "o3", &mut non_default, &mut optional, no_drop())
        .await
        .expect("a null temperature is omitted");
    assert_eq!(
        optional.get("max_completion_tokens"),
        Some(&Value::from(4_i64))
    );
    assert!(!non_default.contains_key("max_tokens"));
    assert!(!optional.contains_key("temperature"));

    let mut kept = request(r#"{"temperature": 1, "max_tokens": 2}"#);
    let mut stored = request("{}");
    map_o3(fixture, "o3", &mut kept, &mut stored, no_drop())
        .await
        .expect("temperature 1 survives");
    assert_eq!(stored.get("temperature"), Some(&Value::from(1_i64)));
    assert_eq!(
        stored.get("max_completion_tokens"),
        Some(&Value::from(2_i64))
    );
}

async fn temperature_rejection(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"temperature": 0.2}"#);
    let mut optional = request("{}");
    let failure = map_o3(fixture, "o3", &mut non_default, &mut optional, no_drop())
        .await
        .expect_err("o-series models only accept temperature 1");
    assert_eq!(failure.status_code(), 400);

    let mut carried = request(r#"{"temperature": 0.2}"#);
    let mut kept = request(r#"{"temperature": 0.2}"#);
    map_o3(fixture, "o3", &mut carried, &mut kept, request_drop())
        .await
        .expect("drop_params removes the temperature");
    assert!(!carried.contains_key("temperature"));
    assert_eq!(kept.get("temperature"), Some(&Value::from(0.2_f64)));

    let mut global = request(r#"{"temperature": 0.2}"#);
    let mut empty = request("{}");
    map_o3(fixture, "o3", &mut global, &mut empty, global_drop())
        .await
        .expect("the global drop removes the temperature");
    assert!(!global.contains_key("temperature"));
}

async fn sampling_omitted(fixture: &mut Fixture) {
    let mut non_default = request(
        r#"{"reasoning_effort": "high", "logprobs": true, "top_p": 0.5,
            "presence_penalty": 0.1, "frequency_penalty": 0.1, "top_logprobs": 2}"#,
    );
    let mut optional = request("{}");
    map_o3(fixture, "o3", &mut non_default, &mut optional, no_drop())
        .await
        .expect("o-series sampling params are omitted, not rejected");
    assert_eq!(optional.get("reasoning_effort"), Some(&Value::from("high")));
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
