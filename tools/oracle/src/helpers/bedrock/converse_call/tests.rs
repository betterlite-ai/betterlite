#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::fixture::Input;

const OPERATION: &str = "bedrock_converse_call";
const SEED_PREFIX: &str = "bedrock_converse_call";

#[test]
fn helper_seeds_select_bedrock_converse_call() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the corpus folder is readable")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(SEED_PREFIX))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed is readable");
            (path, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no {SEED_PREFIX} seed exists");
    assert!(seeds.iter().all(|(path, source)| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("a seed parses");
        validate_helper(path, &case).is_ok()
            && case.call.operation == OPERATION
            && serde_json::from_value::<Input>(case.call.input).is_ok()
    }));
}
