#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_config::sections::mistral_options::Settings as EnvironmentSettings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use tracing::subscriber::with_default;

use super::{
    DEFAULT_API_BASE, MistralSettings, REASONING_SYSTEM_PROMPT, ReasoningEffortSelector,
    STANDALONE_MAX_RECURSE_DEPTH, clean_tool_schema_for_mistral, map_openai_params, provider_info,
    remove_json_schema_refs, supported_openai_params,
};
use crate::base_helpers::provider_config;

struct NoEfforts;

impl ReasoningEffortSelector for NoEfforts {
    fn accepted_effort(&self, _model: &str, _requested: &str) -> Option<String> {
        None
    }
}

struct RoundsMedium;

impl ReasoningEffortSelector for RoundsMedium {
    fn accepted_effort(&self, model: &str, requested: &str) -> Option<String> {
        match (model, requested) {
            ("mistral-small-latest", "medium") => Some(String::from("high")),
            ("mistral-small-latest", other) => Some(other.to_owned()),
            (_, _) => None,
        }
    }
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("a JSON value")
}

fn object(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).expect("a JSON object")
}

fn settings() -> EnvironmentSettings {
    EnvironmentSettings {
        azure_api_base: None,
        azure_api_key: None,
        api_key: None,
        max_recurse_depth: 100,
    }
}

fn map(
    params: &[(&str, Value)],
    model: &str,
    selector: &dyn ReasoningEffortSelector,
    max_depth: i64,
) -> Map<String, Value> {
    let pairs: Vec<(String, Value)> = params
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    let mut optional = Map::new();
    map_openai_params(&pairs, &mut optional, model, selector, max_depth);
    optional
}

fn base_params() -> Vec<&'static str> {
    vec![
        "stream",
        "temperature",
        "top_p",
        "max_tokens",
        "max_completion_tokens",
        "tools",
        "tool_choice",
        "seed",
        "stop",
        "response_format",
        "parallel_tool_calls",
    ]
}

#[test]
fn supported_params_follow_magistral_and_reasoning_capability() {
    let mut with_effort = base_params();
    with_effort.push("reasoning_effort");
    let mut magistral = with_effort.clone();
    magistral.insert(11, "thinking");
    assert_eq!(
        supported_openai_params("mistral-small-latest", false),
        base_params()
    );
    assert_eq!(
        supported_openai_params("mistral-small-latest", true),
        with_effort
    );
    assert_eq!(
        supported_openai_params("Mistral-Magistral-2506", false),
        magistral
    );
    assert_eq!(
        supported_openai_params("magistral-small-latest", true),
        magistral
    );
}

#[test]
fn class_defaults_stay_absent() {
    let config = provider_config(&MistralSettings::default()).expect("a config object");
    assert!(config.is_empty());
}

#[test]
fn configured_class_values_export_all_nine_supplied_keys() {
    let config = provider_config(&MistralSettings {
        temperature: Some(Value::from(0.4_f64)),
        top_p: Some(Value::from(1_i64)),
        max_tokens: Some(Value::from(64_i64)),
        tools: Some(value(r#"[{"type":"function"}]"#)),
        tool_choice: Some(Value::from("auto")),
        random_seed: Some(Value::from(7_i64)),
        safe_prompt: Some(false),
        response_format: Some(value(r#"{"type":"json_object"}"#)),
        stop: Some(value(r#"["END"]"#)),
    })
    .expect("a config object");
    assert_eq!(config.len(), 9);
    assert_eq!(config.get("temperature"), Some(&Value::from(0.4_f64)));
    assert_eq!(config.get("safe_prompt"), Some(&Value::Bool(false)));
    assert_eq!(config.get("random_seed"), Some(&Value::from(7_i64)));
    assert_eq!(config.get("stop"), Some(&value(r#"["END"]"#)));
}

#[test]
fn token_limits_overwrite_in_iteration_order() {
    let completion_then_tokens = map(
        &[
            ("max_completion_tokens", Value::from(5_i64)),
            ("max_tokens", Value::from(9_i64)),
        ],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(completion_then_tokens, object(r#"{"max_tokens":9}"#));
    let tokens_then_completion = map(
        &[
            ("max_tokens", Value::from(9_i64)),
            ("max_completion_tokens", Value::from(5_i64)),
        ],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(tokens_then_completion, object(r#"{"max_tokens":5}"#));
}

#[rstest]
#[case::literal_true(Value::Bool(true), Some(Value::Bool(true)))]
#[case::integer_one(Value::from(1_i64), None)]
#[case::text_true(Value::from("true"), None)]
#[case::literal_false(Value::Bool(false), None)]
fn stream_passes_only_a_literal_true(#[case] value: Value, #[case] expected: Option<Value>) {
    let mapped = map(
        &[("stream", value)],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(mapped.get("stream"), expected.as_ref());
}

#[rstest]
#[case::auto(Value::from("auto"), Some(Value::from("auto")))]
#[case::none(Value::from("none"), Some(Value::from("none")))]
#[case::any(Value::from("any"), Some(Value::from("any")))]
#[case::required(Value::from("required"), Some(Value::from("any")))]
#[case::empty(Value::from(""), Some(Value::from("any")))]
#[case::object(value(r#"{"type":"function"}"#), None)]
#[case::null(Value::Null, None)]
#[case::number(Value::from(3_i64), None)]
fn tool_choice_maps_strings_and_drops_other_values(
    #[case] choice: Value,
    #[case] expected: Option<Value>,
) {
    let mapped = map(
        &[("tool_choice", choice)],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(mapped.get("tool_choice"), expected.as_ref());
}

#[test]
fn a_non_string_tool_choice_leaves_the_callers_value_alone() {
    let mut optional = object(r#"{"tool_choice":"caller"}"#);
    map_openai_params(
        &[("tool_choice".to_owned(), value(r#"{"type":"function"}"#))],
        &mut optional,
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(optional, object(r#"{"tool_choice":"caller"}"#));
}

#[test]
fn seed_replaces_the_whole_extra_body() {
    let mapped = map(
        &[("seed", Value::from(7_i64))],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(mapped, object(r#"{"extra_body":{"random_seed":7}}"#));
    let mut optional = object(r#"{"extra_body":{"keep":1}}"#);
    map_openai_params(
        &[("seed".to_owned(), Value::from(7_i64))],
        &mut optional,
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(optional, object(r#"{"extra_body":{"random_seed":7}}"#));
}

#[test]
fn copied_params_pass_through_unchanged() {
    let mapped = map(
        &[
            ("temperature", Value::from(0.4_f64)),
            ("top_p", Value::from(1_i64)),
            ("stop", value(r#"["END"]"#)),
            ("response_format", value(r#"{"type":"json_object"}"#)),
            ("parallel_tool_calls", Value::Bool(false)),
            ("bogus", Value::Bool(true)),
        ],
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(
        mapped,
        object(
            r#"{"temperature":0.4,"top_p":1,"stop":["END"],"response_format":{"type":"json_object"},"parallel_tool_calls":false}"#
        )
    );
}

#[test]
fn non_magistral_reasoning_effort_comes_from_the_injected_selector() {
    let rounded = map(
        &[("reasoning_effort", Value::from("medium"))],
        "mistral-small-latest",
        &RoundsMedium,
        100,
    );
    assert_eq!(rounded.get("reasoning_effort"), Some(&Value::from("high")));
    let kept = map(
        &[("reasoning_effort", Value::from("low"))],
        "mistral-small-latest",
        &RoundsMedium,
        100,
    );
    assert_eq!(kept.get("reasoning_effort"), Some(&Value::from("low")));
    let no_declaration = map(
        &[("reasoning_effort", Value::from("medium"))],
        "another-model",
        &RoundsMedium,
        100,
    );
    assert_eq!(
        no_declaration.get("reasoning_effort"),
        Some(&Value::from("medium"))
    );
    let not_text = map(
        &[("reasoning_effort", Value::from(3_i64))],
        "mistral-small-latest",
        &RoundsMedium,
        100,
    );
    assert_eq!(not_text.get("reasoning_effort"), Some(&Value::from(3_i64)));
}

#[test]
fn magistral_reasoning_options_request_the_prompt_instead_of_an_effort() {
    let mapped = map(
        &[
            ("reasoning_effort", Value::from("medium")),
            ("thinking", value(r#"{"type":"enabled"}"#)),
        ],
        "Mistral-MAGISTRAL-2506",
        &RoundsMedium,
        100,
    );
    assert_eq!(mapped, object(r#"{"_add_reasoning_prompt":true}"#));
}

#[test]
fn a_rounded_effort_is_logged_once_and_an_unchanged_one_is_not() {
    let log = tempfile::NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(log.reopen().expect("a reopened log"))
        .finish();
    with_default(subscriber, || {
        let rounded = map(
            &[("reasoning_effort", Value::from("medium"))],
            "mistral-small-latest",
            &RoundsMedium,
            100,
        );
        assert_eq!(rounded.get("reasoning_effort"), Some(&Value::from("high")));
        let unchanged = map(
            &[("reasoning_effort", Value::from("low"))],
            "mistral-small-latest",
            &RoundsMedium,
            100,
        );
        assert_eq!(unchanged.get("reasoning_effort"), Some(&Value::from("low")));
    });
    let rendered = fs::read_to_string(log.path()).expect("the log");
    assert_eq!(rendered.matches("DEBUG").count(), 1);
    assert!(rendered.contains("mistral-small-latest"));
    assert!(rendered.contains("medium"));
    assert!(rendered.contains("high"));
}

#[test]
fn schema_cleanup_removes_references_and_keeps_strict_fields() {
    let tools = value(
        r#"[{"type":"function","function":{"name":"f","strict":true,"additionalProperties":false,"parameters":{"$id":"top","$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"x":{"$id":"inner","type":"string"}}}}}]"#,
    );
    let cleaned = clean_tool_schema_for_mistral(&tools, 100);
    assert_eq!(
        cleaned.pointer("/0/function/strict"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        cleaned.pointer("/0/function/additionalProperties"),
        Some(&Value::Bool(false))
    );
    assert!(cleaned.pointer("/0/function/parameters/$id").is_none());
    assert!(cleaned.pointer("/0/function/parameters/$schema").is_none());
    assert!(
        cleaned
            .pointer("/0/function/parameters/properties/x/$id")
            .is_none()
    );
    assert_eq!(
        tools.pointer("/0/function/parameters/$id"),
        Some(&Value::from("top"))
    );
}

#[test]
fn schema_cleanup_stops_at_the_remaining_depth_boundary() {
    let tools = value(
        r#"[{"$id":"tool","function":{"$schema":"fn","parameters":{"$id":"params","properties":{"leaf":{"$id":"leaf"}}}}}]"#,
    );
    let at_three = clean_tool_schema_for_mistral(&tools, 3);
    assert!(at_three.pointer("/0/$id").is_none());
    assert!(at_three.pointer("/0/function/$schema").is_none());
    assert_eq!(
        at_three.pointer("/0/function/parameters/$id"),
        Some(&Value::from("params"))
    );
    let at_four = clean_tool_schema_for_mistral(&tools, 4);
    assert!(at_four.pointer("/0/function/parameters/$id").is_none());
    assert_eq!(
        at_four.pointer("/0/function/parameters/properties/leaf/$id"),
        Some(&Value::from("leaf"))
    );
    assert_eq!(clean_tool_schema_for_mistral(&tools, 1), tools);
    assert_eq!(clean_tool_schema_for_mistral(&tools, 0), tools);
    assert_eq!(clean_tool_schema_for_mistral(&tools, -2), tools);
}

#[test]
fn the_standalone_utility_processes_exactly_ten_levels() {
    assert_eq!(STANDALONE_MAX_RECURSE_DEPTH, 10);
    let mut schema = value(
        r#"{"$id":"d10","next":{"$id":"d9","next":{"$id":"d8","next":{"$id":"d7","next":{"$id":"d6","next":{"$id":"d5","next":{"$id":"d4","next":{"$id":"d3","next":{"$id":"d2","next":{"$id":"d1","next":{"$id":"d0"}}}}}}}}}}}"#,
    );
    remove_json_schema_refs(&mut schema, STANDALONE_MAX_RECURSE_DEPTH);
    assert!(schema.get("$id").is_none());
    assert_eq!(
        schema.pointer("/next/next/next/next/next/next/next/next/next/$id"),
        Some(&Value::from("d1"))
    );
    assert_eq!(
        schema.pointer("/next/next/next/next/next/next/next/next/next/next/$id"),
        Some(&Value::from("d0"))
    );
}

#[test]
fn map_cleans_tools_with_the_supplied_depth() {
    let params = [("tools".to_owned(), value(r#"[{"$id":"tool"}]"#))];
    let mut shallow = Map::new();
    map_openai_params(&params, &mut shallow, "m", &NoEfforts, 1);
    assert_eq!(shallow.get("tools"), Some(&value(r#"[{"$id":"tool"}]"#)));
    let mut deeper = Map::new();
    map_openai_params(&params, &mut deeper, "m", &NoEfforts, 2);
    assert_eq!(deeper.get("tools"), Some(&value("[{}]")));
}

#[test]
fn unknown_params_never_reach_the_options() {
    let mut optional = object(r#"{"keep":1}"#);
    map_openai_params(
        &[("bogus".to_owned(), Value::Bool(true))],
        &mut optional,
        "mistral-small-latest",
        &NoEfforts,
        100,
    );
    assert_eq!(optional, object(r#"{"keep":1}"#));
}

#[test]
fn provider_info_prefers_explicit_values_then_azure_then_defaults() {
    let empty = settings();
    let (default_base, default_key) = provider_info(None, None, &empty);
    assert_eq!(default_base, DEFAULT_API_BASE);
    assert!(default_key.is_none());

    let azure = EnvironmentSettings {
        azure_api_base: Some(String::from("https://azure.example")),
        azure_api_key: Some(SecretString::from("azure-key")),
        api_key: Some(SecretString::from("mistral-key")),
        max_recurse_depth: 100,
    };
    let (azure_base, azure_key) = provider_info(None, None, &azure);
    assert_eq!(azure_base, "https://azure.example/v1");
    assert_eq!(
        azure_key.expect("an azure key").expose_secret(),
        "azure-key"
    );

    let (explicit_base, explicit_key) =
        provider_info(Some("https://given.example/v1"), Some("given"), &azure);
    assert_eq!(explicit_base, "https://given.example/v1");
    assert_eq!(
        explicit_key.expect("an explicit key").expose_secret(),
        "given"
    );

    let (slashed_base, slashed_key) =
        provider_info(Some("https://given.example/v1/"), None, &empty);
    assert_eq!(slashed_base, "https://given.example/v1//v1");
    assert!(slashed_key.is_none());

    let mistral_key = EnvironmentSettings {
        azure_api_base: None,
        azure_api_key: None,
        api_key: Some(SecretString::from("mistral-key")),
        max_recurse_depth: 100,
    };
    let (fallback_base, fallback_key) = provider_info(None, None, &mistral_key);
    assert_eq!(fallback_base, DEFAULT_API_BASE);
    assert_eq!(
        fallback_key.expect("a mistral key").expose_secret(),
        "mistral-key"
    );
}

#[test]
fn the_reasoning_prompt_is_the_bundled_fixture() {
    let bundled = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/mistral_reasoning.txt"),
    )
    .expect("the bundled prompt");
    assert_eq!(REASONING_SYSTEM_PROMPT, bundled);
    assert!(REASONING_SYSTEM_PROMPT.starts_with('\n'));
}
