#![cfg(test)]

use betterlite_models::decisions::Json;

use super::{compiled_settings, template_metadata};
use crate::arize::phoenix::parsing::parse_prompt_data;
use crate::arize::phoenix::records::PromptTemplate;

fn template(data: &str) -> PromptTemplate {
    parse_prompt_data(serde_json::from_str(data).unwrap(), "v1").unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[test]
fn metadata_takes_model_temperature_and_max_tokens_from_nested_parameters() {
    let found = template_metadata(&template(
        r#"{"model_name":"gpt-4o","invocation_parameters":{"openai":{"temperature":0.5,"max_tokens":256}}}"#,
    ));
    assert_eq!(found["model"], json(r#""gpt-4o""#));
    assert_eq!(found["temperature"], json("0.5"));
    assert_eq!(found["max_tokens"], json("256"));
}

#[test]
fn metadata_merges_provider_parameters_the_template_does_not_already_hold() {
    let found = template_metadata(&template(
        r#"{"model_name":"gpt-4o","invocation_parameters":{"openai":{"model":"other","temperature":0.5,"seed":7,"top_p":0.9}}}"#,
    ));
    assert_eq!(found["model"], json(r#""gpt-4o""#));
    assert_eq!(found["seed"], json("7"));
    assert_eq!(found["top_p"], json("0.9"));
    assert_eq!(
        found.keys().map(String::as_str).collect::<Vec<_>>(),
        ["model", "temperature", "max_tokens", "seed", "top_p"]
    );
}

#[test]
fn metadata_reads_extra_parameters_from_the_anthropic_block_when_openai_is_absent() {
    let found = template_metadata(&template(
        r#"{"invocation_parameters":{"anthropic":{"max_tokens":10,"top_p":0.4}}}"#,
    ));
    assert_eq!(found["max_tokens"], json("10"));
    assert_eq!(found["top_p"], json("0.4"));
}

#[test]
fn metadata_does_not_merge_extras_from_the_first_nested_fallback_block() {
    let found = template_metadata(&template(
        r#"{"invocation_parameters":{"vertex":{"temperature":0.7,"top_p":0.4}}}"#,
    ));
    assert_eq!(found["temperature"], json("0.7"));
    assert!(!found.contains_key("top_p"));
}

#[test]
fn metadata_keeps_missing_values_as_null() {
    let found = template_metadata(&template("{}"));
    assert_eq!(found["model"], Json::Null);
    assert_eq!(found["temperature"], Json::Null);
    assert_eq!(found["max_tokens"], Json::Null);
}

#[test]
fn metadata_compiled_settings_pick_only_the_optional_parameters() {
    let settings = compiled_settings(&template(
        r#"{"model_name":"gpt-4o","invocation_parameters":{"openai":{"temperature":0.5,"max_tokens":256,"top_p":0.9,"frequency_penalty":0.1,"presence_penalty":0.2,"seed":7}}}"#,
    ));
    assert_eq!(settings.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        settings
            .optional_params
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "temperature",
            "max_tokens",
            "top_p",
            "frequency_penalty",
            "presence_penalty"
        ]
    );
    assert_eq!(settings.optional_params["presence_penalty"], json("0.2"));
}

#[test]
fn metadata_compiled_settings_carry_absent_temperature_and_max_tokens_as_null() {
    let settings = compiled_settings(&template("{}"));
    assert_eq!(settings.model, None);
    assert_eq!(settings.optional_params["temperature"], Json::Null);
    assert_eq!(settings.optional_params["max_tokens"], Json::Null);
    assert_eq!(settings.optional_params.len(), 2);
}
