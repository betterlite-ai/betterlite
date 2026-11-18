#![cfg(test)]

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{ORACLE_DIR, run_case};
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_anthropic_provider() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("anthropic_provider")
        })
        .collect();
    assert!(!seeds.is_empty(), "no anthropic_provider seeds");
    assert!(seeds.iter().all(|path| {
        let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        validate_helper(path, &case).unwrap();
        case.call.operation == "anthropic_provider"
            && case
                .calls
                .iter()
                .all(|call| call.operation == "anthropic_provider")
            && run_case(&[crate::GROUP], &case, false).is_ok()
    }));
}
