use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::spend::estimates::decode::decode_spend_estimate;

#[rstest]
fn replays_the_spend_estimate_corpus(#[files("fuzz/corpus/spend_estimate/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus seed is readable");
    drop(decode_spend_estimate(&bytes));
}
