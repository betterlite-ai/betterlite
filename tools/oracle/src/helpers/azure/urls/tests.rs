#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 8] = [
    "credential_precedence",
    "deployment_base_url",
    "v1_latest_preview",
    "query_api_version_wins",
    "append_route_once",
    "azure_header_selection",
    "azure_env_none_vs_empty",
    "v1_image_resource_reset",
];

#[test]
fn helper_seeds_select_azure_urls() {
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
        .filter(|(name, _source)| name.starts_with("azure_urls"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_urls seeds");
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
        assert_eq!(case.call.operation, "azure_urls");
        assert!(
            VARIANTS.contains(&variant.as_str()),
            "{name} names {variant}"
        );
        assert!(validate_helper(Path::new("<helper>"), &case).is_ok());
        seen.insert(variant);
        true
    });
    assert!(valid);
    assert_eq!(seen.len(), VARIANTS.len());
}
