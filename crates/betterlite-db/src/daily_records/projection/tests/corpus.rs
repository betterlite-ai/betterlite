#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::daily_records::metadata::{decode_daily_metadata, request_status};

// Every seed runs through both readers, the strict builder and the permissive status classifier, so no byte string panics either.
#[rstest]
fn replays_the_daily_metadata_corpus(#[files("fuzz/corpus/daily_metadata/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _metadata = decode_daily_metadata(&bytes);
    let _status = request_status(Some(&bytes));
}
