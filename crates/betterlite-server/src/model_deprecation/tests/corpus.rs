use std::fs;
use std::path::PathBuf;
use std::str;

use chrono::Datelike as _;
use rstest::rstest;

use crate::model_deprecation::dates::decode_deprecation_date;

#[rstest]
fn replays_the_model_deprecation_corpus(#[files("fuzz/corpus/model_deprecation/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus seed is readable");
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    if let Some(date) = decode_deprecation_date(text) {
        assert!((1..=9999).contains(&date.year()));
    }
}
