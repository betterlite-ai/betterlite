#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

fn corpus_paths() -> Vec<PathBuf> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn selects(source: &str) -> bool {
    let Ok(case) = serde_saphyr::from_str::<HelperCase>(source) else {
        return false;
    };
    validate_helper(Path::new("<helper>"), &case).is_ok()
        && case.call.operation == "bedrock_converse_response"
        && Op::check(&case.call.input).is_ok()
}

#[test]
fn helper_seeds_select_bedrock_converse_response() {
    let seeds: Vec<String> = corpus_paths()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_converse_response"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| selects(source)));
}
