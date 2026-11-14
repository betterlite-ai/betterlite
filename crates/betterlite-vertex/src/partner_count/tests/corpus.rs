use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::partner_count::response::decode_partner_count;

#[rstest]
fn replays_the_partner_count_corpus(#[files("fuzz/corpus/partner_count/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_partner_count(&bytes));
}
