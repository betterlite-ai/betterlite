#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;

#[test]
fn helper_seeds_select_google_pricing() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("google_pricing"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "google_pricing"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn the_operation_reaches_the_production_character_pricing() {
    let input: super::input::Input = serde_json::from_value(json!({
        "operation": "cost_per_character",
        "model": "gemini-1.5-pro-002",
        "custom_llm_provider": "vertex_ai",
        "record": {
            "input_cost_per_character": 0.000000625,
            "input_cost_per_character_above_128k_tokens": 0.00000125,
            "output_cost_per_character": 0.0000025,
        },
        "usage": {"prompt_tokens": 100, "completion_tokens": 20},
        "prompt_characters": 32001,
        "completion_characters": 40,
    }))
    .unwrap();
    let value = super::ops::run(&input).unwrap();
    assert_eq!(
        value,
        json!({
            "prompt_cost": "0.04000125",
            "completion_cost": "0.0001",
            "calls": [],
        })
    );
}

#[test]
fn the_operation_records_a_token_fallback_and_its_missing_location() {
    let input: super::input::Input = serde_json::from_value(json!({
        "operation": "cost_per_character",
        "model": "gemini-1.5-pro-002",
        "custom_llm_provider": "vertex_ai",
        "record": {"output_cost_per_character": 0.5, "regional_endpoint_uplift_multiplier": 1.25},
        "usage": {"prompt_tokens": 5, "completion_tokens": 10},
        "completion_characters": 10,
        "service_tier": "priority",
        "vertex_location": "us-east5",
    }))
    .unwrap();
    let value = super::ops::run(&input).unwrap();
    assert_eq!(
        value,
        json!({
            "prompt_cost": "6.25",
            "completion_cost": "6.25",
            "calls": [{
                "model": "gemini-1.5-pro-002",
                "provider": "vertex_ai",
                "service_tier": "priority",
                "vertex_location": null,
                "prompt_tokens": 5,
                "completion_tokens": 10,
            }],
        })
    );
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::input::Input>(
        json!({"operation": "cost_router", "unexpected": 1}),
    );
    assert!(unknown.unwrap_err().to_string().contains("unexpected"));
}

#[test]
fn a_missing_record_reports_the_failure() {
    let input: super::input::Input = serde_json::from_value(json!({
        "operation": "web_search_cost",
        "usage": {"prompt_tokens": 1, "completion_tokens": 0},
    }))
    .unwrap();
    let failure = super::ops::run(&input).unwrap_err();
    assert!(failure.message.contains("record"));
}
