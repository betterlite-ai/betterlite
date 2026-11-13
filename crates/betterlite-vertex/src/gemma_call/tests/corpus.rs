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
    if name.starts_with("embedded-429") {
        let status = outcome.err().map(|error| error.status_code());
        assert_eq!(status, Some(429), "{name}");
        return;
    }
    if name.starts_with("missing-predictions") {
        let status = outcome.err().map(|error| error.status_code());
        assert_eq!(status, Some(422), "{name}");
        return;
    }
    assert!(outcome.is_err(), "{name}");
}
