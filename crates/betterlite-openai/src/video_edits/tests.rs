#![cfg(test)]

mod cases;
mod character;
mod edit;
mod extension;
mod media;
mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::video_edits::response::decode_character_reply;

#[rstest]
fn replays_the_openai_video_edits_corpus(
    #[files("fuzz/corpus/openai_video_edits/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("corpus seed");
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .expect("utf-8 file name");
    let decoded = decode_character_reply(&bytes);
    if name.starts_with("valid") || name.starts_with("boundary") {
        assert!(decoded.is_ok(), "{name}: {decoded:?}");
    } else {
        assert!(decoded.is_err(), "{name} decoded");
    }
}
