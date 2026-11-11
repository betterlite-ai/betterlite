#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;

use super::Op;
use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_model_capability_predicates() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("model_capability_predicates-")?
                .to_owned();
            Some((stem, fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no model_capability_predicates seeds");
    let checked = seeds
        .iter()
        .map(|(stem, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            assert_eq!(case.call.operation, "model_capability_predicates", "{stem}");
            Op::check(&case.call.input).unwrap();
            let input = case.call.input.as_object().unwrap();
            assert_eq!(input.len(), 2, "{stem}: fixture and steps");
            assert!(!input["steps"].as_array().unwrap().is_empty(), "{stem}");
            stem.as_str()
        })
        .count();
    assert_eq!(checked, seeds.len());
}
