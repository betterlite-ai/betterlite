#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_vertex_gcs_access() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry
                .file_name()
                .as_encoded_bytes()
                .starts_with(b"vertex_gcs_access")
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
        assert_eq!(case.call.operation, "vertex_gcs_access");
        let run = crate::helpers::run_case(&case, false).unwrap();
        matches!(run, crate::helpers::HelperRun::Match)
    }));
}
