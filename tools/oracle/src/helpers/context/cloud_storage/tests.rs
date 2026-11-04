#![cfg(test)]

use crate::case::{HelperCase, helper::validate_helper};
use std::fs;
use std::path::Path;
#[test]
fn helper_seeds_select_cloud_storage() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            let name = entry.file_name();
            name.as_encoded_bytes().starts_with(b"cloud_storage")
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
        assert_eq!(case.call.operation, "cloud_storage");
        let run = crate::helpers::run_case(&case, false).unwrap();
        matches!(run, crate::helpers::HelperRun::Match)
    }));
}
