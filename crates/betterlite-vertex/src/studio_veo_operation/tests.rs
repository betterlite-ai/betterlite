#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::videos::reply::VideoObjectType;
use rstest::rstest;

use super::decode::decode_studio_veo_operation;
use super::download::download_target;
use super::records::status_response;

// the seeds are named for the decoder path they exercise, so a new seed states its expectation in its name
#[rstest]
fn replays_the_studio_veo_operation_corpus(
    #[files("fuzz/corpus/studio_veo_operation/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a readable seed");
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .expect("a utf-8 seed name");
    let decoded = decode_studio_veo_operation(&bytes);
    if name.starts_with("malformed") {
        assert!(decoded.is_err(), "{name}");
        return;
    }
    assert!(decoded.is_ok(), "{name}");
    let video = status_response(&bytes, Some("gemini")).expect("every operation projects a status");
    assert_eq!(video.object, VideoObjectType::Video, "{name}");
    let target = download_target(&bytes);
    if name.starts_with("target-") {
        assert!(target.is_ok(), "{name}");
        return;
    }
    assert!(target.is_err(), "{name}");
}
