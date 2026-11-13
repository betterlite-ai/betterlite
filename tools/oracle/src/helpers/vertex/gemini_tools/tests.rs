#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

use super::{Input, Op};

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_tools"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

#[test]
fn replays_the_gemini_tools_helper_cases() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "gemini_tools"
    }));
}

#[test]
fn the_gemini_tools_seed_parses_into_a_mapping_call() {
    let seeds = seed_sources();
    let inputs: Vec<Input> = seeds
        .iter()
        .map(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
            serde_json::from_value(case.call.input).expect("seed input should decode")
        })
        .collect();
    assert!(
        inputs.iter().any(|input| {
            !input.call.tools.is_empty() && input.call.web_search_options.is_some()
        })
    );
}

fn input(call: &Value) -> Input {
    serde_json::from_value(json!({"call": call, "settings": {}})).expect("input should decode")
}

#[test]
fn the_adapter_maps_native_tools_choices_and_tiers() {
    let input = input(&json!({
        "tools": [
            {"google_search": {"searchType": "SEARCH_TYPE_MEDIA"}},
            {"googleMaps": {"latitude": 1.5_f64, "longitude": 2.5_f64, "languageCode": "en"}}
        ],
        "functions": [],
        "web_search_options": null,
        "tool_choice": "auto",
        "service_tier": "AUTO",
        "server_side_invocations": false,
        "tool_config": null
    }));
    let output = Op::run(&input).expect("adapter should map the tools");
    assert_eq!(
        output,
        json!({
            "tools": [
                {"googleSearch": {"searchType": "SEARCH_TYPE_MEDIA"}},
                {"googleMaps": {}}
            ],
            "tool_choice": {"functionCallingConfig": {"mode": "AUTO"}},
            "toolConfig": {"retrievalConfig": {"latLng": {"latitude": 1.5_f64, "longitude": 2.5_f64}, "languageCode": "en"}},
            "service_tier": "priority",
            "warnings": 0_usize
        })
    );
}

#[test]
fn the_adapter_reports_the_search_conflict_warning() {
    let input = input(&json!({
        "tools": [
            {"type": "function", "function": {"name": "f"}},
            {"googleSearch": {}}
        ],
        "functions": [],
        "web_search_options": {},
        "tool_choice": null,
        "service_tier": null,
        "server_side_invocations": false,
        "tool_config": null
    }));
    let output = Op::run(&input).expect("adapter should map the tools");
    assert_eq!(
        output,
        json!({
            "tools": [{"function_declarations": [{"name": "f"}]}],
            "tool_choice": null,
            "toolConfig": null,
            "service_tier": null,
            "warnings": 2_usize
        })
    );
}

#[test]
fn the_adapter_leaves_functions_list_parameters_unshaped() {
    let input = input(&json!({
        "tools": [],
        "functions": [{
            "name": "f",
            "parameters": {
                "type": "object",
                "$defs": {"City": {"type": "string"}},
                "properties": {"city": {"$ref": "#/$defs/City"}}
            }
        }],
        "web_search_options": null,
        "tool_choice": null,
        "service_tier": null,
        "server_side_invocations": false,
        "tool_config": null
    }));
    let output = Op::run(&input).expect("adapter should map the functions");
    assert_eq!(
        output,
        json!({
            "tools": [{"function_declarations": [{
                "name": "f",
                "parameters": {
                    "type": "object",
                    "$defs": {"City": {"type": "string"}},
                    "properties": {"city": {"$ref": "#/$defs/City"}}
                }
            }]}],
            "tool_choice": null,
            "toolConfig": null,
            "service_tier": null,
            "warnings": 0_usize
        })
    );
}

#[test]
fn the_adapter_reports_an_unsupported_tool_choice_with_status_400() {
    let input = input(&json!({
        "tools": [],
        "functions": [],
        "web_search_options": null,
        "tool_choice": "mandatory",
        "service_tier": null,
        "server_side_invocations": false,
        "tool_config": null
    }));
    let error = Op::run(&input).expect_err("mandatory is not a supported choice");
    assert_eq!(error.status, Some(400));
    assert_eq!(error.class, "UnsupportedParamsError");
}
