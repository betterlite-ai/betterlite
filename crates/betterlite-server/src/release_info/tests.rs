#![cfg(test)]

mod bullets;
mod composition;
mod records;
mod route;
mod service;
mod source;
mod support;

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::records::{LatestRelease, decode_fuzzed_latest_release};

#[rstest]
fn replays_the_latest_release_corpus(#[files("fuzz/corpus/latest_release/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus seed should be readable");
    let decoded = decode_fuzzed_latest_release(&bytes);
    let again = decode_fuzzed_latest_release(&bytes);
    assert_eq!(decoded, again);
    if bytes.first().is_some_and(|status| *status != 200) {
        assert!(matches!(decoded, LatestRelease::Unavailable(_)));
    }
}
