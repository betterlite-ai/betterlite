#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::gemini_live_client::decode::decode_gemini_live_client_event;

#[rstest]
fn replays_the_gemini_live_client_corpus(
    #[files("fuzz/corpus/gemini_live_client/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the seed is readable");
    drop(decode_gemini_live_client_event(&bytes, usize::MAX));
}
