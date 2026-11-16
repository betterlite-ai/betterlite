#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

const PREDICT_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/imagen-3.0-generate-002:predict";

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_imagen_generation"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

fn decode(source: &str) -> Input {
    let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
    assert_eq!(case.call.operation, "vertex_imagen_generation");
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
fn helper_seeds_select_vertex_imagen_generation() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let inputs: Vec<Input> = sources.iter().map(|source| decode(source)).collect();
    assert!(inputs.iter().any(|input| !input.mappings.is_empty()));
    assert!(inputs.iter().any(|input| input.response.is_some()));
}

#[test]
fn a_custom_base_is_trimmed_and_skips_authentication() {
    let input = input(json!({
        "model": "imagen-3.0-generate-002",
        "mappings": [],
        "target": target("https://proxy.example/predict///"),
        "response": null,
    }));
    let value = Op::run(&input).expect("a custom base needs no token");
    assert_eq!(
        value["target"]["url"],
        json!("https://proxy.example/predict")
    );
    assert_eq!(
        value["target"]["headers"],
        json!({"x-goog-user-project": "billing"})
    );
}

#[test]
fn an_empty_base_predicts_the_standard_url_without_authentication() {
    let input = input(json!({
        "model": "imagen-3.0-generate-002",
        "mappings": [],
        "target": target(""),
        "response": null,
    }));
    let value = Op::run(&input).expect("an empty base still skips the token");
    assert_eq!(value["target"]["url"], json!(PREDICT_URL));
    assert_eq!(
        value["target"]["headers"],
        json!({"x-goog-user-project": "billing"})
    );
}

#[test]
fn labels_from_the_request_metadata_reach_the_body_root() {
    let input = input(json!({
        "model": "imagen-3.0-generate-002",
        "mappings": [{
            "prompt": "a red fox",
            "non_default_params": {"n": 2_i32},
            "optional_params": {},
            "litellm_params": {"metadata": {"requester_metadata": {"team": "a"}}},
        }],
        "target": null,
        "response": null,
    }));
    let value = Op::run(&input).expect("the labels come from the metadata");
    assert_eq!(
        value["mappings"][0]["mapped"],
        json!({"sampleCount": 2_i32})
    );
    assert_eq!(
        value["mappings"][0]["body"]["parameters"],
        json!({"sampleCount": 2_i32})
    );
    assert_eq!(value["mappings"][0]["body"]["labels"], json!({"team": "a"}));
}

#[test]
fn a_reply_appends_only_the_predictions_with_the_encoded_key() {
    let input = input(json!({
        "model": "imagen-3.0-generate-002",
        "mappings": [],
        "target": null,
        "response": {
            "predictions": [
                {"raiFilteredReason": "blocked"},
                {"bytesBase64Encoded": "AAA"},
                {"bytesBase64Encoded": ""},
                {"bytesBase64Encoded": null},
            ],
        },
    }));
    let value = Op::run(&input).expect("the reply transforms");
    assert_eq!(
        value["response"]["response"],
        json!({
            "data": [{"b64_json": "AAA"}, {"b64_json": ""}, {}],
            "usage": {
                "input_tokens": 0_i32,
                "input_tokens_details": {"image_tokens": 0_i32, "text_tokens": 0_i32},
                "output_tokens": 0_i32,
                "total_tokens": 0_i32,
            },
        })
    );
}
