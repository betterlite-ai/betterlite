#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::cases::GENERATED_ID;
use crate::rerank::decode::decode_rerank_reply;

#[rstest]
fn replays_the_bedrock_other_rerank_corpus(
    #[files("fuzz/corpus/bedrock_other_rerank/*")] path: PathBuf,
) {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let accepted = name.starts_with("valid") || name.starts_with("boundary");
    assert_eq!(
        decode_rerank_reply(&bytes, GENERATED_ID).is_ok(),
        accepted,
        "{name}"
    );
    if name == "valid-usage.json" {
        let response = decode_rerank_reply(&bytes, GENERATED_ID).unwrap();
        let meta = response.meta.as_ref().unwrap();
        assert_eq!(
            meta.billed_units
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .search_units,
            Some(Some(2))
        );
    }
}
