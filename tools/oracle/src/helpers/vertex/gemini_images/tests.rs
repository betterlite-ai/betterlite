#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

const STANDARD_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash-image:generateContent";

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_gemini_images"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

fn decode(source: &str) -> Input {
    let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
    assert_eq!(case.call.operation, "vertex_gemini_images");
    serde_json::from_value(case.call.input).expect("seed input should decode")
}

fn input(value: Value) -> Input {
    serde_json::from_value(value).expect("the helper input should decode")
}

fn target(api_base: &str) -> Value {
    json!({
        "api_base": api_base,
        "params_api_base": null,
        "project": "proj",
        "location": "us-central1",
        "headers": {"x-goog-user-project": "billing"},
    })
}

#[test]
fn helper_seeds_select_vertex_gemini_images() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let inputs: Vec<Input> = sources.iter().map(|source| decode(source)).collect();
    assert!(inputs.iter().any(|input| !input.mappings.is_empty()));
    assert!(inputs.iter().any(|input| input.response.is_some()));
}

#[test]
fn a_custom_base_is_trimmed_and_skips_authentication() {
    let input = input(json!({
        "model": "gemini-2.5-flash-image",
        "mappings": [],
        "target": target("https://proxy.example/v1beta///"),
        "response": null,
    }));
    let value = Op::run(&input).expect("a custom base needs no token");
    assert_eq!(
        value["target"]["url"],
        json!("https://proxy.example/v1beta")
    );
    assert_eq!(
        value["target"]["headers"],
        json!({"x-goog-user-project": "billing"})
    );
}

#[test]
fn an_empty_base_builds_the_standard_url_without_authentication() {
    let input = input(json!({
        "model": "gemini-2.5-flash-image",
        "mappings": [],
        "target": target(""),
        "response": null,
    }));
    let value = Op::run(&input).expect("an empty base still skips the token");
    assert_eq!(value["target"]["url"], json!(STANDARD_URL));
    assert_eq!(
        value["target"]["headers"],
        json!({"x-goog-user-project": "billing"})
    );
}

#[test]
fn a_non_object_image_config_only_counts_a_warning() {
    let input = input(json!({
        "model": "gemini-2.5-flash-image",
        "mappings": [{
            "prompt": "a red fox",
            "non_default_params": {"imageConfig": 3_i32},
            "optional_params": {},
        }],
        "target": null,
        "response": null,
    }));
    let value = Op::run(&input).expect("the mapping warns instead of failing");
    assert_eq!(value["mappings"][0]["warnings"], json!(1_i32));
    assert_eq!(value["mappings"][0]["mapped"], json!({}));
    assert_eq!(
        value["mappings"][0]["body"]["generationConfig"],
        json!({"responseModalities": ["IMAGE"]})
    );
}

#[test]
fn a_reply_without_usage_keeps_the_search_count_out_of_the_response() {
    let input = input(json!({
        "model": "gemini-2.5-flash-image",
        "mappings": [],
        "target": null,
        "response": {
            "candidates": [
                {"groundingMetadata": {"webSearchQueries": ["a", "b"]}},
                {"groundingMetadata": [{"webSearchQueries": ["b"]}]},
                {"content": {"parts": [{"inlineData": {"data": "AAA"}}]}},
            ],
        },
    }));
    let value = Op::run(&input).expect("the reply transforms");
    assert_eq!(value["response"]["search_requests"], json!(2_i32));
    assert_eq!(
        value["response"]["response"],
        json!({"data": [{"b64_json": "AAA"}]})
    );
}
