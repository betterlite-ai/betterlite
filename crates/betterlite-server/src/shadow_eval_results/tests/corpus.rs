#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::{ShadowQueryError, decode_shadow_query};

#[rstest]
fn replays_the_shadow_q_corpus(#[files("fuzz/corpus/shadow_q/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let parsed = decode_shadow_query(&bytes);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    if name.starts_with("shadow-eval-results-valid") {
        assert_eq!(parsed.unwrap().limit, 10);
    } else {
        assert_eq!(
            parsed.unwrap_err(),
            ShadowQueryError::Invalid {
                parameter: "limit",
                kind: "greater_than_equal",
                msg: "Input should be greater than or equal to 1",
            }
        );
    }
}
