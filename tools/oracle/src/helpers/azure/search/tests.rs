#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 9] = [
    "request",
    "headers",
    "citations",
    "max_results",
    "cap",
    "reply",
    "error",
    "settings",
    "price",
];

#[test]
fn helper_seeds_select_azure_search() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
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
        .filter(|(name, _source)| name.starts_with("azure_search"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_search seeds");
    let variants: BTreeSet<String> = seeds
        .iter()
        .map(|(name, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            assert_eq!(case.call.operation, "azure_search");
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
