#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;
use crate::helpers::Helper as _;

#[test]
fn helper_seeds_select_gemini_response() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_response"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "gemini_response"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn a_missing_usage_metadata_is_a_vertex_422() {
    let input: super::Input = serde_json::from_value(json!({
        "model": "requested-model",
        "legacy_function": false,
        "now": 1_700_000_000_i64,
        "generated_id": "0123456789abcdef0123456789abcdef",
        "headers": {},
        "body": {"candidates": [{"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"}]},
    }))
    .unwrap();
    let failure = super::Op::run(&input).unwrap_err();
    assert_eq!(failure.status, Some(422));
}
