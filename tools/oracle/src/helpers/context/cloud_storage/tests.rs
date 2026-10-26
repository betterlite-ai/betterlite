#![cfg(test)]
use crate::case::{HelperCase, parse_helper_case};
use std::fs;
#[test]
fn helper_seeds_select_cloud_storage() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.file_name().unwrap().to_str().unwrap().starts_with("cloud_storage")).collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).unwrap();
        parse_helper_case(&source).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "cloud_storage");
        matches!(crate::helpers::run_case(&case, false).unwrap(), crate::helpers::HelperRun::Match)
    }));
}
