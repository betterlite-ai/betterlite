#![cfg(test)]

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_request_records() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("request_records")
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    assert!(!seeds.is_empty());
    assert!(seeds.into_iter().all(|source| {
        serde_saphyr::from_str(&source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        case.call.operation == "request_records"
    }));
}
