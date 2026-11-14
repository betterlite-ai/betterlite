#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn run(value: Value) -> Result<Value, String> {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).map_err(|error| error.class)
}

#[test]
fn helper_seeds_select_vertex_batch_target() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_batch_target"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "vertex_batch_target"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(
        json!({"api_base": null, "project": "p", "location": "global", "model": "m", "other": 1_i32}),
    );
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn the_production_targets_are_returned() {
    let value = run(json!({
        "api_base": "https://proxy.example.com/v1",
        "project": "proj",
        "location": "us-central1",
        "model": "projects/proj/locations/us-central1/endpoints/222",
    }))
    .unwrap();
    assert_eq!(
        value,
        json!({
            "collection": "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs",
            "endpoint": "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/222",
        })
    );
}

#[test]
fn an_invalid_location_is_a_value_error() {
    let error = run(json!({
        "api_base": null,
        "project": "proj",
        "location": "Evil.example",
        "model": "projects/proj/locations/x/endpoints/1",
    }))
    .unwrap_err();
    assert_eq!(error, "ValueError");
}
