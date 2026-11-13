#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 7] = [
    "default_api_version",
    "cloudflare_base_model",
    "separate_token_client_identity",
    "azure_sdk_client_mode",
    "azure_initializer_credentials",
    "azure_v1_client_projection",
    "dynamic_api_version",
];

#[test]
fn helper_seeds_select_azure_endpoints() {
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
        .filter(|(name, _source)| name.starts_with("azure_endpoints"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_endpoints seeds");
    let mut seen = BTreeSet::new();
    let valid = seeds.iter().all(|(name, source)| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let variant = case
            .call
            .input
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        assert_eq!(case.call.operation, "azure_endpoints");
        assert!(
            VARIANTS.contains(&variant.as_str()),
            "{name} names {variant}"
        );
        validate_helper(Path::new("<helper>"), &case).unwrap();
        seen.insert(variant);
        true
    });
    assert!(valid);
    assert_eq!(seen.len(), VARIANTS.len());
}
