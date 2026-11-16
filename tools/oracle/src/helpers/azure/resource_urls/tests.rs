#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 5] = [
    "container_api_version_query",
    "container_normalized_base",
    "video_supported_fields",
    "video_auth_and_url",
    "vector_search_azure_base",
];

#[test]
fn helper_seeds_select_azure_resource_urls() {
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
        .filter(|(name, _source)| name.starts_with("azure_resource_urls"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_resource_urls seeds");
    let mut seen = BTreeSet::new();
    for (name, source) in &seeds {
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
        assert_eq!(case.call.operation, "azure_resource_urls");
        assert!(
            VARIANTS.contains(&variant.as_str()),
            "{name} names {variant}"
        );
        validate_helper(Path::new("<helper>"), &case).unwrap();
        seen.insert(variant);
    }
    assert_eq!(seen.len(), VARIANTS.len());
}
