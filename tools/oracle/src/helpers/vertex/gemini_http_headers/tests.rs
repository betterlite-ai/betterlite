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
                .is_some_and(|name| name.starts_with("gemini_http_headers"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

#[test]
fn replays_the_gemini_http_headers_seeds() {
    let seeds = seed_sources();
    assert!(
        !seeds.is_empty(),
        "the corpus holds a gemini_http_headers seed"
    );
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gemini_http_headers"
    }));
}

#[test]
fn the_gemini_http_headers_seeds_reach_the_projection() {
    let outputs: Vec<Value> = seed_sources()
        .iter()
        .map(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
            let input: Input =
                serde_json::from_value(case.call.input).expect("seed input should decode");
            Op::run(&input).expect("every seed projects headers")
        })
        .collect();
    assert!(
        outputs.iter().any(|output| {
            output.pointer("/use_psc_endpoint_format") == Some(&Value::Bool(true))
        })
    );
    assert!(
        outputs
            .iter()
            .any(|output| output.pointer("/headers/Authorization").is_some())
    );
    assert!(
        outputs.iter().any(|output| {
            output.pointer("/headers/x-goog-api-key") == Some(&json!("helper-key"))
        })
    );
}

fn input(call: &Value) -> Input {
    serde_json::from_value(call.clone()).expect("input should decode")
}

#[test]
fn the_projection_keeps_the_content_type_under_a_bearer() {
    let output = Op::run(&input(&json!({
        "api_key": "ya29.token",
        "auth_map": null,
        "headers": null,
        "optional_params": {}
    })))
    .expect("the token projects a bearer");
    assert_eq!(
        output,
        json!({
            "headers": {
                "Authorization": "Bearer ya29.token",
                "Content-Type": "application/json"
            },
            "use_psc_endpoint_format": false
        })
    );
}

#[test]
fn the_projection_applies_the_caller_headers_last() {
    let output = Op::run(&input(&json!({
        "api_key": null,
        "auth_map": {"x-goog-api-key": "auth-key"},
        "headers": {"x-goog-api-key": "caller-key", "Content-Type": "text/plain"},
        "optional_params": {}
    })))
    .expect("the caller overrides the map auth");
    assert_eq!(
        output,
        json!({
            "headers": {
                "Content-Type": "text/plain",
                "x-goog-api-key": "caller-key"
            },
            "use_psc_endpoint_format": false
        })
    );
}

#[test]
fn the_projection_reads_the_psc_flag() {
    let output = Op::run(&input(&json!({
        "api_key": "ya29.token",
        "auth_map": null,
        "headers": null,
        "optional_params": {"use_psc_endpoint_format": true}
    })))
    .expect("the flag projects");
    assert_eq!(output["use_psc_endpoint_format"], json!(true));
}

#[test]
fn the_projection_refuses_a_textual_psc_flag() {
    let error = Op::run(&input(&json!({
        "api_key": "ya29.token",
        "auth_map": null,
        "headers": null,
        "optional_params": {"use_psc_endpoint_format": "yes"}
    })))
    .expect_err("a string flag is not a boolean");
    assert_eq!(error.class, "ValueError");
    assert!(error.message.contains("use_psc_endpoint_format"));
}
