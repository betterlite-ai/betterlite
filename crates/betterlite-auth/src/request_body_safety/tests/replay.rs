use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::request_body_safety::decode_request_metadata;

#[rstest]
fn replays_the_request_body_safety_corpus(
    #[files("fuzz/corpus/request_body_safety/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus seed is readable");
    drop(decode_request_metadata(&bytes));
}
