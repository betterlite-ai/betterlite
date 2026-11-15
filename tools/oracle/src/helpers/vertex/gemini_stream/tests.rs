#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;
use crate::helpers::Helper as _;

#[test]
fn helper_seeds_select_gemini_stream() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_stream-"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "gemini_stream"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn an_embedded_error_reports_its_status() {
    let input: super::Input = serde_json::from_value(json!({
        "fragments": ["{\"error\":{\"code\":\"429\",\"status\":\"RESOURCE_EXHAUSTED\"}}"],
        "legacy_function": false,
        "headers": {},
    }))
    .unwrap();
    let failure = super::Op::run(&input).unwrap_err();
    assert_eq!(failure.status, Some(429));
    assert_eq!(failure.message, "RESOURCE_EXHAUSTED - Unknown error");
}
