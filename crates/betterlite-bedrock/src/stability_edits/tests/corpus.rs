#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::stability_edits::reply::decode_stability_reply;

#[rstest]
fn replays_the_bedrock_other_stability_edits_corpus(
    #[files("fuzz/corpus/bedrock_other_stability_edits/*")] path: PathBuf,
) {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let accepted = name.starts_with("valid") || name.starts_with("boundary");
    assert_eq!(decode_stability_reply(&bytes).is_ok(), accepted, "{name}");
    if name == "valid-images.json" {
        let artifacts = decode_stability_reply(&bytes).unwrap();
        assert_eq!(artifacts.images, ["aGVsbG8=", "d29ybGQ="]);
        assert!(artifacts.errors.is_none());
        assert!(artifacts.finish_reason.is_none());
    }
}
