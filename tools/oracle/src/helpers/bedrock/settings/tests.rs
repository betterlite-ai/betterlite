#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_bedrock_settings() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("bedrock_settings")) {
            seeds.push(fs::read_to_string(path).unwrap());
        }
    }
    assert!(!seeds.is_empty());
    for source in seeds {
        parse_helper_case(&source).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "bedrock_settings");
    }
}
