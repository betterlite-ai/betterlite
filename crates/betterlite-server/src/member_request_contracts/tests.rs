#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::Value;

use super::decode_all;

#[rstest]
fn replays_the_member_request_contracts_corpus(
    #[files("fuzz/corpus/member_request_contracts/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        decode_all(&value);
    }
}
