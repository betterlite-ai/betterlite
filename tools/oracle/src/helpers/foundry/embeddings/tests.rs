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
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_embeddings"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty(), "no foundry_embeddings helper case seeds");
    assert!(seeds.iter().all(|case| {
        validate_helper(Path::new("<helper>"), case).is_ok()
            && case.call.operation == "foundry_embeddings"
            && Op::check(&case.call.input).is_ok()
            && case
                .call
                .input
                .as_object()
                .and_then(|input| input.get("flows"))
                .and_then(|flows| flows.as_array())
                .is_some_and(|flows| !flows.is_empty())
    }));
}
