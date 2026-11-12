use betterlite_openai::chat::{Stop, ToolChoice};
use rstest::rstest;
use serde_json::json;

use super::prelude::{translate, translate_with};
use crate::api::genai_chat_input::input::GenerateLiteLLMParams;

#[test]
fn generation_config_maps_to_the_completion_options() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "generationConfig": {
            "temperature": 0.25_f64,
            "maxOutputTokens": 64_i32,
            "topP": 0.9_f64,
            "stopSequences": ["END"],
            "topK": 5_i32,
        },
    }));
    assert_eq!(
        request.temperature.and_then(|value| value.as_f64()),
        Some(0.25_f64)
    );
    assert_eq!(request.max_tokens, Some(64));
    assert_eq!(
        request.top_p.and_then(|value| value.as_f64()),
        Some(0.9_f64)
    );
    assert_eq!(request.stop, Some(Stop::Many(vec![String::from("END")])));
    assert!(request.passthrough.is_empty());
}

#[test]
fn an_absent_generation_config_sets_no_options() {
    let request = translate(json!({"model": "m", "contents": []}));
    assert_eq!(request.temperature, None);
    assert_eq!(request.max_tokens, None);
    assert_eq!(request.top_p, None);
    assert_eq!(request.stop, None);
}

#[test]
fn a_null_generation_config_sets_no_options() {
    let request = translate(json!({"model": "m", "contents": [], "generationConfig": null}));
    assert_eq!(request.temperature, None);
    assert_eq!(request.max_tokens, None);
}

#[rstest]
#[case::auto("AUTO", "auto")]
#[case::any("ANY", "required")]
#[case::none("NONE", "none")]
#[case::unknown("SOMETIMES", "auto")]
#[case::lowercase("auto", "auto")]
fn tool_config_modes_map_to_tool_choice(#[case] mode: &str, #[case] expected: &str) {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "toolConfig": {"functionCallingConfig": {"mode": mode}},
    }));
    assert_eq!(
        request.tool_choice,
        Some(ToolChoice::Mode(String::from(expected)))
    );
}

#[test]
fn a_tool_config_without_a_mode_defaults_to_auto() {
    let request = translate(json!({"model": "m", "contents": [], "toolConfig": {}}));
    assert_eq!(
        request.tool_choice,
        Some(ToolChoice::Mode(String::from("auto")))
    );
}

#[test]
fn a_null_tool_config_sets_no_tool_choice() {
    let request = translate(json!({"model": "m", "contents": [], "toolConfig": null}));
    assert_eq!(request.tool_choice, None);
}

#[test]
fn litellm_params_are_returned_beside_the_completion_request() {
    let given = GenerateLiteLLMParams {
        api_base: Some(String::from("https://example.test")),
        api_key: Some(secrecy::SecretString::from("sk-test")),
        api_version: Some(String::from("2024-01-01")),
        custom_llm_provider: Some(String::from("openai")),
        max_retries: Some(2),
        organization: None,
    };
    let (request, forwarded) = translate_with(
        json!({"model": "m", "contents": [], "generationConfig": {"temperature": 1_i32}}),
        &given,
    );
    assert_eq!(request.max_tokens, None);
    let params = forwarded.expect("the forwarded params");
    assert_eq!(params.api_base.as_deref(), Some("https://example.test"));
    assert_eq!(
        params
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-test")
    );
    assert_eq!(params.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(params.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(params.max_retries, Some(2));
}

#[test]
fn absent_litellm_params_stay_absent() {
    let (_, forwarded) = translate_with(
        json!({"model": "m", "contents": []}),
        &GenerateLiteLLMParams {
            api_base: None,
            api_key: None,
            api_version: None,
            custom_llm_provider: None,
            max_retries: None,
            organization: None,
        },
    );
    let params = forwarded.expect("the forwarded params");
    assert_eq!(params.api_base, None);
    assert!(params.api_key.is_none());
    assert_eq!(params.api_version, None);
    assert_eq!(params.custom_llm_provider, None);
    assert_eq!(params.max_retries, None);
}

#[test]
fn the_snake_case_tool_config_is_read() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "tool_config": {"functionCallingConfig": {"mode": "NONE"}},
    }));
    assert_eq!(
        request.tool_choice,
        Some(ToolChoice::Mode(String::from("none")))
    );
}

#[test]
fn the_camel_case_tool_config_wins_over_the_snake_case_one() {
    let request = translate(json!({
        "model": "m",
        "contents": [],
        "toolConfig": {"functionCallingConfig": {"mode": "ANY"}},
        "tool_config": {"functionCallingConfig": {"mode": "NONE"}},
    }));
    assert_eq!(
        request.tool_choice,
        Some(ToolChoice::Mode(String::from("required")))
    );
}
