#![cfg(test)]

use super::Op;
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_json_provider_registry() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("json_provider_registry"))
        })
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "json_provider_registry");
        validate_helper(Path::new("<helper>"), &case).unwrap();
        Op::decode_and_run(&case.call.input).is_ok()
    }));
}
