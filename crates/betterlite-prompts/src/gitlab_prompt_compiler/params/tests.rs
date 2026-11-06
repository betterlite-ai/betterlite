#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use indexmap::IndexMap;

use crate::arguments::json::Json;
use crate::gitlab_templates::parse::{FrontmatterParser, parse_prompt_file};
use crate::gitlab_templates::template::GitLabPromptTemplate;

use super::{project_optional_params, prompt_metadata};

fn template(content: &str) -> GitLabPromptTemplate {
    parse_prompt_file(content, "id", FrontmatterParser::Yaml).unwrap()
}

fn keys<Value>(object: &IndexMap<String, Value>) -> Vec<&str> {
    object.keys().map(String::as_str).collect()
}

#[test]
fn the_metadata_carries_the_model_and_always_temperature_and_max_tokens() {
    let metadata = prompt_metadata(&template("---\nmodel: gpt-4o\n---\nUser: hi"));

    assert_eq!(keys(&metadata), vec!["model", "temperature", "max_tokens"]);
    assert_eq!(metadata.get("model").and_then(Json::as_str), Some("gpt-4o"));
    assert_eq!(metadata.get("temperature"), Some(&Json::Null));
    assert_eq!(metadata.get("max_tokens"), Some(&Json::Null));
}

#[test]
fn optional_params_overlay_the_primary_fields() {
    let metadata = prompt_metadata(&template(
        "---\ntemperature: 0.5\ntop_p: 0.9\n---\nUser: hi",
    ));

    assert_eq!(
        keys(&metadata),
        vec!["model", "temperature", "max_tokens", "top_p"]
    );
    assert_eq!(
        serde_json::to_string(metadata.get("temperature").unwrap()).unwrap(),
        "0.5"
    );
    assert_eq!(
        serde_json::to_string(metadata.get("top_p").unwrap()).unwrap(),
        "0.9"
    );
}

#[test]
fn the_five_parameters_are_projected_in_order() {
    let metadata = prompt_metadata(&template(
        "---\ntemperature: 0.5\nmax_tokens: 100\ntop_p: 0.9\nfrequency_penalty: 0.1\npresence_penalty: 0.2\nstop: [x]\n---\nUser: hi",
    ));

    let projected = project_optional_params(&metadata);

    assert_eq!(
        keys(&projected),
        vec![
            "temperature",
            "max_tokens",
            "top_p",
            "frequency_penalty",
            "presence_penalty"
        ]
    );
}

#[test]
fn only_temperature_and_max_tokens_are_projected_without_metadata() {
    let metadata = prompt_metadata(&template("User: hi"));

    let projected = project_optional_params(&metadata);

    assert_eq!(keys(&projected), vec!["temperature", "max_tokens"]);
    assert!(projected.values().all(|value| *value == ContractJson::Null));
}

#[test]
fn a_null_parameter_that_is_present_is_projected_as_null() {
    let metadata = prompt_metadata(&template("---\ntop_p: null\n---\nUser: hi"));

    let projected = project_optional_params(&metadata);

    assert_eq!(keys(&projected), vec!["temperature", "max_tokens", "top_p"]);
    assert_eq!(projected.get("top_p"), Some(&ContractJson::Null));
}
