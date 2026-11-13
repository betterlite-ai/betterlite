#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const OPERATIONS: [&str; 9] = [
    "unsupported_region_warning",
    "llama_openapi",
    "ai21_stream",
    "host_and_version_grafts",
    "psc_required_fields",
    "gemini_custom_key",
    "query_preserved",
    "header_override",
    "cache_passthrough_path",
];

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_endpoint"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

#[test]
fn helper_seeds_select_vertex_endpoint() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "vertex_endpoint"
    }));
}

#[test]
fn helper_seeds_select_every_vertex_endpoint_operation() {
    let keys: Vec<String> = seed_sources()
        .into_iter()
        .flat_map(|source| {
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            let keys: Vec<String> = case
                .call
                .input
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            keys
        })
        .collect();
    assert!(
        OPERATIONS
            .iter()
            .all(|operation| keys.iter().any(|key| key.as_str() == *operation))
    );
}
