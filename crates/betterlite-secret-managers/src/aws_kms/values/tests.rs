#![cfg(test)]

use super::{decode_ciphertext, parse_plaintext};
use crate::aws_kms::DecryptedValue;
use rstest::rstest;

#[test]
fn prefix_stripping_removes_only_the_leading_aws_kms_marker() {
    assert_eq!(decode_ciphertext("aws_kms/aGVsbG8=").unwrap(), b"hello");
    assert_eq!(decode_ciphertext("aGVsbG8=").unwrap(), b"hello");
}

#[rstest]
#[case("true")]
#[case("TRUE")]
#[case("false")]
#[case(" FALSE ")]
#[case("1")]
fn boolean_parsing_only_accepts_python_case_sensitive_literals(#[case] plaintext: &str) {
    assert!(matches!(
        parse_plaintext(plaintext),
        DecryptedValue::String(_)
    ));
}

#[test]
fn python_boolean_literals_become_boolean_values() {
    assert_eq!(parse_plaintext(" True "), DecryptedValue::Boolean(true));
    assert_eq!(parse_plaintext("False"), DecryptedValue::Boolean(false));
}
