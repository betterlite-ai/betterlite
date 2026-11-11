#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_bedrock_services_batch_create() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_batch_create"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let parsed = serde_saphyr::from_str::<HelperCase>(source).expect("seed should parse");
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|case| {
                validate_helper(Path::new("<helper>"), &case).map_err(|error| error.to_string())
            })
            .is_ok()
            && parsed.call.operation == "bedrock_services_batch_create"
    }));
}
