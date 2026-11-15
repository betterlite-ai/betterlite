#![cfg(test)]

mod cases;
mod character;
mod edit;
mod extension;
mod media;
mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use crate::video_edits::response::decode_character_reply;

#[test]
fn replays_the_openai_video_edits_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_video_edits");
    let mut seeds = 0_u32;
    for entry in fs::read_dir(corpus).expect("corpus folder") {
        let path = entry.expect("corpus entry").path();
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
        seeds = seeds.saturating_add(1);
    }
    assert!(seeds > 0, "the corpus has no seeds");
}
