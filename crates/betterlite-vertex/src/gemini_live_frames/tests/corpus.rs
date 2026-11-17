#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::gemini_live_frames::decode::decode_gemini_live_frame_bytes;

#[rstest]
fn replays_the_gemini_live_frames_corpus(
    #[files("fuzz/corpus/gemini_live_frames/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the seed is readable");
    drop(decode_gemini_live_frame_bytes(&bytes, usize::MAX));
}
