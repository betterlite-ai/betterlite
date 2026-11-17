#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

#[test]
fn helper_seeds_select_foundry_rerank() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the helper corpus exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_rerank"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(!seeds.is_empty(), "no foundry_rerank seed exists");
    assert!(seeds.iter().all(|source| {
        let parsed = serde_saphyr::from_str::<HelperCase>(source).expect("a seed parses");
        validate_helper(Path::new("<foundry_rerank>"), &parsed).is_ok()
            && parsed.call.operation == "foundry_rerank"
            && Op::check(&parsed.call.input).is_ok()
    }));
}
