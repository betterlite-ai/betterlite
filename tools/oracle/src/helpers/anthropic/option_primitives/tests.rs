#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 6] = [
    "defaults",
    "model_output_capacity",
    "stop_sequence_filter",
    "web_search_context_size",
    "context_compaction_edit",
    "user_identifier_filter",
];

#[test]
fn helper_seeds_select_anthropic_option_primitives() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anthropic_option_primitives"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .map(|source| serde_saphyr::from_str(&source).unwrap())
        .collect();
    assert!(!seeds.is_empty(), "no anthropic_option_primitives seeds");
    assert!(seeds.iter().all(|case| {
        let calls = case.call.input.get("calls").and_then(Value::as_array);
        case.call.operation == "anthropic_option_primitives"
            && calls.is_some_and(|entries| {
                !entries.is_empty()
                    && entries.iter().all(|call| {
                        call.get("function")
                            .and_then(Value::as_str)
                            .is_some_and(|name| VARIANTS.contains(&name))
                    })
            })
            && validate_helper(Path::new("<helper>"), case).is_ok()
    }));
}
