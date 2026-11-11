#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use crate::s3_transport::error_code::{error_code, is_terminal};

#[rstest]
fn replays_the_s3_error_classifier_corpus(
    #[files("fuzz/corpus/s3_error_classifier/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("corpus file");
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    let code = error_code(text);
    for status in [400_u16, 403, 429, 500] {
        let terminal = is_terminal(status, text);
        if terminal {
            assert!(matches!(status, 400 | 403));
            assert!(code.is_some());
        }
    }
}
