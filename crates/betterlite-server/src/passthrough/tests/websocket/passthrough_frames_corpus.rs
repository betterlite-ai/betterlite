use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use super::rewriter;
use crate::passthrough::websocket::{json_object_frame, rewrite_live_setup_model};

#[rstest]
fn replays_the_passthrough_frames_corpus(
    #[files("fuzz/corpus/passthrough_frames/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("corpus seed is readable");
    drop(json_object_frame(&bytes));
    if let Ok(text) = from_utf8(&bytes) {
        drop(rewrite_live_setup_model(text, Some(&rewriter)));
    }
}
