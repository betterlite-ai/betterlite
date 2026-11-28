#![cfg(test)]

mod cookie_policy;
mod origin_cases;
mod return_cache_order;
mod support;

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::login_return::origin::validate_return_to;
use crate::login_return::return_destination::{
    MAX_RETURN_DESTINATION_BYTES, ReturnDestinationError, decode_return_destination,
    is_same_origin_return_path,
};

#[test]
fn return_destination_limit() {
    let at_limit = vec![b'a'; MAX_RETURN_DESTINATION_BYTES];
    let decoded = decode_return_destination(&at_limit).expect("the byte bound is inclusive");
    assert_eq!(decoded.len(), MAX_RETURN_DESTINATION_BYTES);
    let over = vec![b'a'; MAX_RETURN_DESTINATION_BYTES.saturating_add(1)];
    let error = decode_return_destination(&over)
        .expect_err("a destination past the byte bound is rejected");
    assert_eq!(
        error,
        ReturnDestinationError::TooLong {
            length: MAX_RETURN_DESTINATION_BYTES.saturating_add(1)
        }
    );
    let invalid = decode_return_destination(b"/ui/\xff").expect_err("invalid UTF-8 is rejected");
    assert!(matches!(invalid, ReturnDestinationError::NotUtf8(_)));
}

#[rstest]
fn replays_the_login_return_corpus(#[files("fuzz/corpus/login_return/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus seed is readable");
    let destination =
        decode_return_destination(&bytes).expect("the seed is UTF-8 within the byte bound");
    let _relative = is_same_origin_return_path(&destination);
    let _configured = validate_return_to(&destination, Some("https://cp.example"));
    let _unconfigured = validate_return_to(&destination, None);
}
