#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::NvidiaNimConfig;
use crate::openai_base_config::Object;

fn object(value: Value) -> Object {
    match value {
        Value::Object(fields) => fields,
        _ => panic!("the fixture is not an object"),
    }
}

fn mapped(model: &str, non_default: Value, optional: Value) -> Object {
    NvidiaNimConfig::map_params(model, &object(non_default), object(optional))
}

const GEMMA: &[&str] = &[
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "stop",
    "seed",
];
const DEFAULT: &[&str] = &[
    "stream",
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "seed",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "response_format",
];

#[rstest]
#[case::recurrentgemma("google/recurrentgemma-2b", GEMMA)]
#[case::gemma_27b("google/gemma-2-27b-it", GEMMA)]
#[case::gemma_9b("google/gemma-2-9b-it", GEMMA)]
#[case::gemma_9b_unprefixed("gemma-2-9b-it", GEMMA)]
#[case::nemotron_instruct(
    "nvidia/nemotron-4-340b-instruct",
    &["stream", "temperature", "top_p", "max_tokens", "max_completion_tokens"]
)]
#[case::nemotron_reward("nvidia/nemotron-4-340b-reward", &["stream"])]
#[case::codegemma(
    "google/codegemma-1.1-7b",
    &[
        "stream",
        "temperature",
        "top_p",
        "frequency_penalty",
        "presence_penalty",
        "max_tokens",
        "max_completion_tokens",
        "stop",
    ]
)]
#[case::default_model("meta/llama3-70b-instruct", DEFAULT)]
fn supported_params_match_the_model_branch(#[case] model: &str, #[case] expected: &[&str]) {
    assert_eq!(NvidiaNimConfig::supported_params(model), expected);
}

#[test]
fn supported_params_match_the_model_exactly() {
    assert_eq!(
        NvidiaNimConfig::supported_params("nvidia_nim/google/gemma-2-9b-it"),
        NvidiaNimConfig::supported_params("meta/llama3-70b-instruct")
    );
    assert_ne!(
        NvidiaNimConfig::supported_params("google/gemma-2-9b-it"),
        NvidiaNimConfig::supported_params("meta/llama3-70b-instruct")
    );
}

#[test]
fn map_params_rewrites_max_completion_tokens_when_the_model_lists_it() {
    let mapped = mapped(
        "nvidia/nemotron-4-340b-instruct",
        json!({"max_completion_tokens": 12}),
        json!({}),
    );
    assert_eq!(mapped, object(json!({"max_tokens": 12})));
}

#[test]
fn map_params_rewrites_max_completion_tokens_for_a_model_that_does_not_list_it() {
    let mapped = mapped(
        "nvidia/nemotron-4-340b-reward",
        json!({"max_completion_tokens": 7}),
        json!({}),
    );
    assert_eq!(mapped, object(json!({"max_tokens": 7})));
}

#[test]
fn map_params_drops_a_param_the_model_does_not_support() {
    let mapped = mapped(
        "nvidia/nemotron-4-340b-reward",
        json!({"seed": 3, "stream": true}),
        json!({}),
    );
    assert_eq!(mapped, object(json!({"stream": true})));
}

#[test]
fn map_params_copies_every_supported_param() {
    let params = json!({
        "stream": true,
        "temperature": 0.5,
        "top_p": 0.9,
        "frequency_penalty": 0.1,
        "presence_penalty": 0.2,
        "max_tokens": 20,
        "stop": ["end"],
        "seed": 4,
        "tools": [],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "response_format": {"type": "json_object"}
    });
    let mapped = mapped("meta/llama3-70b-instruct", params.clone(), json!({}));
    assert_eq!(mapped, object(params));
}

#[test]
fn map_params_keeps_the_options_the_caller_already_mapped() {
    let mapped = mapped(
        "google/gemma-2-9b-it",
        json!({"temperature": 0.2}),
        json!({"max_tokens": 99, "extra": "kept"}),
    );
    assert_eq!(
        mapped,
        object(json!({"temperature": 0.2, "max_tokens": 99, "extra": "kept"}))
    );
}

#[test]
fn map_params_lets_max_tokens_win_over_max_completion_tokens() {
    let mapped = mapped(
        "meta/llama3-70b-instruct",
        json!({"max_completion_tokens": 2, "max_tokens": 1}),
        json!({}),
    );
    assert_eq!(mapped, object(json!({"max_tokens": 1})));
}

#[test]
fn map_params_returns_the_given_map_when_nothing_is_mapped() {
    let mapped = mapped("nvidia/nemotron-4-340b-reward", json!({}), json!({}));
    assert!(mapped.is_empty());
}

#[test]
fn map_params_leaves_the_requested_map_alone() {
    let non_default = object(json!({"stream": true, "seed": 9}));
    let mapped =
        NvidiaNimConfig::map_params("nvidia/nemotron-4-340b-reward", &non_default, Object::new());
    assert_eq!(mapped, object(json!({"stream": true})));
    assert_eq!(non_default, object(json!({"stream": true, "seed": 9})));
}
