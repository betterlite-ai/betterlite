#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_options"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

#[test]
fn replays_the_gemini_options_seeds() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty(), "the corpus holds a gemini_options seed");
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gemini_options"
    }));
}

fn input(call: &Value) -> Input {
    serde_json::from_value(call.clone()).expect("input should decode")
}

#[test]
fn the_adapter_maps_sampling_and_the_gemini_three_default() {
    let input = input(&json!({
        "safety_settings": null,
        "steps": [{
            "op": "options",
            "model": "gemini-3-pro-preview",
            "non_default_params": {"temperature": 0.5_f64, "top_p": 0.9_f64},
            "optional_params": {}
        }]
    }));
    let output = Op::run(&input).expect("the adapter maps the request");
    assert_eq!(
        output,
        json!({"results": [{
            "options": {"temperature": 0.5_f64, "top_p": 0.9_f64},
            "warnings": 1_usize
        }]})
    );
}

#[test]
fn the_adapter_replaces_the_request_safety_settings() {
    let input = input(&json!({
        "safety_settings": {"category": "CONFIGURED"},
        "steps": [{
            "op": "options",
            "model": "gemini-2.5-pro",
            "non_default_params": {"temperature": 0_i32},
            "optional_params": {"safety_settings": [{"category": "REQUEST"}]}
        }]
    }));
    let output = Op::run(&input).expect("the adapter maps the request");
    assert_eq!(
        output,
        json!({"results": [{
            "options": {"safety_settings": {"category": "CONFIGURED"}, "temperature": 0_i32},
            "warnings": 0_usize
        }]})
    );
}

#[test]
fn the_adapter_maps_auth_aliases() {
    let input = input(&json!({
        "safety_settings": null,
        "steps": [{
            "op": "auth",
            "non_default_params": {"project": "my-project", "region_name": "us-central1"},
            "optional_params": {"vertex_project": "old"}
        }]
    }));
    let output = Op::run(&input).expect("the adapter maps the auth aliases");
    assert_eq!(
        output,
        json!({"results": [{
            "options": {"vertex_project": "my-project", "vertex_location": "us-central1"},
            "warnings": 0_usize
        }]})
    );
}
