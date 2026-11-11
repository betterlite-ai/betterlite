#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const OPERATIONS: [&str; 6] = [
    "systemone_to_request",
    "request_roundtrip",
    "response_to_ir",
    "ir_to_response",
    "parse_response",
    "settings",
];

#[test]
fn helper_seeds_select_openai_decisions() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_decisions"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let operations = case.call.input.as_object().unwrap();
        !operations.is_empty()
            && operations
                .keys()
                .all(|operation| OPERATIONS.contains(&operation.as_str()))
            && case.call.operation == "openai_decisions"
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
