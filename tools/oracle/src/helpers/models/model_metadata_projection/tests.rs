#![cfg(test)]

use crate::case::{HelperCase, parse_helper_case};
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_model_metadata_projection() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("model_metadata_projection"))
        })
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        case.call.operation == "model_metadata_projection" && parse_helper_case(&source).is_ok()
    }));
}
