#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;

use super::Op;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[test]
fn helper_seeds_select_foundry_embeddings() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?.to_owned();
            name.starts_with("foundry_embeddings")
                .then(|| (name, fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no foundry_embeddings helper case seeds");
    let checked = seeds
        .iter()
        .map(|(name, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            assert_eq!(case.call.operation, "foundry_embeddings");
            Op::check(&case.call.input).unwrap();
            let input = case.call.input.as_object().unwrap();
            let flows = input["flows"].as_array().unwrap();
            assert!(!flows.is_empty(), "{name} has no flows");
        })
        .count();
    assert_eq!(checked, seeds.len());
}
