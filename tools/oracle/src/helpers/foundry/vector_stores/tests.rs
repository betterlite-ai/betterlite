#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 7] = [
    "store_type_endpoints",
    "auth_credentials",
    "search_query_fields",
    "vector_query_with_embedding",
    "document_score_citation",
    "unsupported_store_create",
    "azure_search_endpoint",
];

#[test]
fn helper_seeds_select_foundry_vector_stores() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read_to_string(path).unwrap())
        })
        .filter(|(name, _source)| name.starts_with("foundry_vector_stores"))
        .collect();
    assert!(!seeds.is_empty(), "no foundry_vector_stores seeds");
    let variants: BTreeSet<String> = seeds
        .iter()
        .map(|(name, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            assert_eq!(case.call.operation, "foundry_vector_stores");
            let variant = case
                .call
                .input
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone();
            assert!(
                VARIANTS.contains(&variant.as_str()),
                "{name} names {variant}"
            );
            validate_helper(Path::new("<helper>"), &case).unwrap();
            variant
        })
        .collect();
    assert_eq!(variants.len(), VARIANTS.len());
}
