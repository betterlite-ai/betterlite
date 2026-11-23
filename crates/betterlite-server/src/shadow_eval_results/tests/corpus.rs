#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::decode_shadow_query;

#[rstest]
fn replays_the_shadow_q_corpus(#[files("fuzz/corpus/shadow_q/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let parsed = decode_shadow_query(&bytes);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    if name.starts_with("shadow-eval-results-valid") {
        drop(parsed.unwrap());
    } else {
        drop(parsed.unwrap_err());
    }
}
