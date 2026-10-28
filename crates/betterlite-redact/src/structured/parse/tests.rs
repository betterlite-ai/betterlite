#![cfg(test)]

use super::{ParseError, parse_payload};
use crate::structured::MAX_WORK;
use crate::structured::{
    mask_credentials_in_payload, mask_sensitive_structure, redact::redact_credentials_in_payload,
};
use rstest::rstest;
use std::{fs, path::PathBuf};

#[rstest]
#[case::malformed(b"{", false)]
#[case::trailing(b"{}secret", false)]
#[case::invalid_utf8(b"\xff", false)]
#[case::byte_limit(&vec![b' '; MAX_WORK.saturating_add(1)], true)]
fn structured_redaction_behavior_tests_behavior(#[case] input: &[u8], #[case] too_large: bool) {
    let error = parse_payload(input).unwrap_err();
    assert_eq!(matches!(error, ParseError::TooLarge), too_large);
    assert!(!error.to_string().contains("secret"));
}

#[rstest]
fn replays_the_structured_redaction_corpus(
    #[files("fuzz/corpus/structured_redaction/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let result = parse_payload(&bytes);
    if path
        .extension()
        .is_some_and(|extension| extension == "invalid")
    {
        assert!(result.is_err(), "{}", path.display());
    } else {
        let value = result.unwrap();
        drop(mask_sensitive_structure(&value));
        drop(mask_credentials_in_payload(&value));
        if let Some(mapping) = value.as_object() {
            drop(redact_credentials_in_payload(mapping));
        }
    }
}

#[test]
fn accepts_exact_byte_limit() {
    let mut bytes = vec![b' '; MAX_WORK];
    *bytes.first_mut().unwrap() = b'0';
    assert_eq!(parse_payload(&bytes).unwrap().to_string(), "0");
}
