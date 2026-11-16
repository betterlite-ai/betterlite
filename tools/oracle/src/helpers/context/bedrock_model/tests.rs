#![cfg(test)]

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_bedrock_model() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_model-"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "bedrock_model"
    }));
}
