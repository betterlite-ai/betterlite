#![cfg(test)]

use crate::case::{HelperCase, parse_helper_case};
use std::fs;
use std::path::Path;

#[test]
fn helper_seeds_select_bedrock_services_file_content() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_file_content"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        parse_helper_case(source).is_ok() && case.call.operation == "bedrock_services_file_content"
    }));
}
