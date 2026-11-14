#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::Op;
use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gemini_embedding_call_plan"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn plan(inputs: &Value) -> Value {
    Op::decode_and_run(&json!({ "inputs": inputs })).unwrap()
}

#[test]
fn helper_seeds_select_gemini_embedding_call_plan() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "gemini_embedding_call_plan"
    }));
}

#[test]
fn flattening_keeps_repeats_and_drops_non_strings() {
    assert_eq!(
        plan(&json!([["files/a", ["files/a", "b"], [1_i64, 2_i64], "c"]])),
        json!({"calls": [{"elements": ["files/a", "files/a", "b", "c"], "has_file_refs": true}]})
    );
}
