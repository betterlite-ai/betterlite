#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::metadata::Lar1Metadata;

mod behaviors;
mod boundaries;
mod support;

#[rstest]
fn replays_the_lar1_meta_corpus(#[files("fuzz/corpus/lar1_meta/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = Lar1Metadata::parse(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
