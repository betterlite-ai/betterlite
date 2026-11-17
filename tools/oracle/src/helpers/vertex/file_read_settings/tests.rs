#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[test]
fn file_read_settings_seeds_decode_and_match() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry
                .file_name()
                .as_encoded_bytes()
                .starts_with(b"vertex_file_read_settings")
        })
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path.path()).unwrap();
        serde_saphyr::from_str(&source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "vertex_file_read_settings");
        let run = run_case(&[crate::GROUP], &case, false).unwrap();
        matches!(run, HelperRun::Match)
    }));
}
