#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn selected_variant(source: &str) -> String {
    let case: HelperCase = serde_saphyr::from_str(source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "bedrock_converse_content");
    let input = case.call.input.as_object().unwrap();
    assert_eq!(input.len(), 1);
    input.keys().next().unwrap().replace('_', "-")
}

#[test]
fn helper_seeds_select_bedrock_converse_content() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("bedrock_converse_content-")?
                .to_owned();
            Some((stem, fs::read_to_string(path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no bedrock_converse_content seeds");
    let selected: Vec<(String, String)> = seeds
        .iter()
        .map(|(stem, source)| (stem.clone(), selected_variant(source)))
        .collect();
    assert!(selected.iter().all(|(stem, variant)| stem == variant));
}
