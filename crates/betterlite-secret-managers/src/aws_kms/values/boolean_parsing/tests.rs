#![cfg(test)]

use crate::aws_kms::DecryptedValue;
use crate::aws_kms::values::parse_plaintext;
use rstest::rstest;

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
