#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::Op;
use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("partner_count_plan"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn plan(cases: &Value) -> Value {
    Op::decode_and_run(&json!({ "cases": cases })).unwrap()
}

#[test]
fn helper_seeds_select_partner_count_plan() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "partner_count_plan"
    }));
}

#[test]
fn a_claude_case_names_its_publisher_and_endpoint() {
    assert_eq!(
        plan(&json!([{
            "model": "claude-sonnet-4-6@default",
            "project": "proj",
            "location": "us-east5",
        }])),
        json!({"cases": [{
            "publisher": "anthropic",
            "endpoint": "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict",
            "stripped": "claude-sonnet-4-6",
        }]})
    );
}

#[test]
fn a_custom_api_base_replaces_the_vertex_host() {
    assert_eq!(
        plan(&json!([{
            "model": "meta/llama-3.1-405b",
            "project": "proj",
            "location": "global",
            "api_base": "https://proxy.example/v1",
        }])),
        json!({"cases": [{
            "publisher": "meta",
            "endpoint": "https://proxy.example/v1/v1/projects/proj/locations/global/publishers/meta/models/count-tokens:rawPredict",
            "stripped": "meta/llama-3.1-405b",
        }]})
    );
}

#[test]
fn an_unknown_model_is_a_value_error() {
    let error = Op::decode_and_run(&json!({"cases": [{
        "model": "gemini-2.5-pro",
        "project": "proj",
        "location": "us-east5",
    }]}))
    .unwrap_err();

    assert_eq!(error.class, "ValueError");
    assert_eq!(error.message, "Unknown partner model: gemini-2.5-pro");
}
