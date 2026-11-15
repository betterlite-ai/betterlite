#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use uuid::Uuid;

use crate::gemma_call::reply::decode_gemma_reply;

#[rstest]
fn replays_the_gemma_reply_corpus(#[files("fuzz/corpus/gemma_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let outcome = decode_gemma_reply(&bytes, Uuid::nil, || 0);
    if name.starts_with("valid-") {
        assert!(outcome.is_ok(), "{name}");
        return;
    }
    let status = outcome.err().map(|error| error.status_code());
    if name.starts_with("embedded-400") {
        assert_eq!(status, Some(400), "{name}");
        return;
    }
    if name.starts_with("embedded-429") {
        assert_eq!(status, Some(429), "{name}");
        return;
    }
    if name.starts_with("missing-predictions") || name.starts_with("non-object-root") {
        assert_eq!(status, Some(422), "{name}");
        return;
    }
    // an out-of-range code is no error record, and a non-record reply is a Vertex 500
    assert_eq!(status, Some(500), "{name}");
}
