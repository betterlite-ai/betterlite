#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;

use super::Op;
use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_model_metadata_resolution() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("model_metadata_resolution-")?
                .to_owned();
            Some((stem, fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no model_metadata_resolution seeds");
    let (stems, scenarios): (Vec<String>, Vec<String>) = seeds
        .iter()
        .map(|(stem, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            assert_eq!(case.call.operation, "model_metadata_resolution");
            Op::check(&case.call.input).unwrap();
            let input = case.call.input.as_object().unwrap();
            assert_eq!(input.len(), 3, "scenario, fixture and queries");
            let scenario = input["scenario"].as_str().unwrap().replace('_', "-");
            (stem.clone(), scenario)
        })
        .unzip();
    assert_eq!(stems, scenarios);
}
