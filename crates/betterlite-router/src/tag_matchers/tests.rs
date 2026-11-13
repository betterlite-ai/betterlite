#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::tag_matchers::document::parse_tag_match;

mod behaviors;
mod boundaries;
mod support;

#[rstest]
fn replays_the_tag_match_corpus(#[files("fuzz/corpus/tag_match/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_tag_match(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not decode");
    } else {
        assert!(outcome.is_ok(), "{name} must decode");
    }
}
