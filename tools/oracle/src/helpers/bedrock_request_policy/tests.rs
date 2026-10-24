#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::HelperCall;

#[test]
fn helper_seeds_select_bedrock_request_policy() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_request_policy"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let calls: Vec<HelperCall> = seeds
        .iter()
        .map(|source| {
            parse_helper_case(source).unwrap();
            serde_saphyr::from_str::<HelperCase>(source).unwrap().call
        })
        .collect();
    assert!(
        calls
            .iter()
            .all(|call| matches!(call, HelperCall::BedrockRequestPolicy(_)))
    );
}
