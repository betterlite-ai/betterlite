use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::redis_spend::codec::decode_tagged_spend_buffer;

// The first byte of every seed carries the record kind: '@' entity, 'A' daily, 'B' window, 'C' log.
#[rstest]
fn replays_the_spend_buffer_corpus(#[files("fuzz/corpus/spend_buffer/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_tagged_spend_buffer(&bytes));
}
