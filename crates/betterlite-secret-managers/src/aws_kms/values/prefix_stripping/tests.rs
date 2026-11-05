#![cfg(test)]

use crate::aws_kms::decode_ciphertext;
use rstest::rstest;

#[rstest]
#[case::plain("aGVsbG8=")]
#[case::prefixed("aws_kms/aGVsbG8=")]
#[case::newline("aws_kms/aGVsbG8=\n")]
#[case::wrapped("aGVs\r\nbG8=\n")]
#[case::ignored_punctuation("aG!Vs bG8=")]
fn prefix_stripping_accepts_python_base64(#[case] input: &str) {
    assert_eq!(decode_ciphertext(input).unwrap(), b"hello");
}

#[test]
fn prefix_stripping_accepts_cli_wrapping() {
    let wrapped = format!("{}\nYQ==\n", "YWFh".repeat(19));
    assert_eq!(decode_ciphertext(&wrapped).unwrap(), vec![b'a'; 58]);
}

#[test]
fn prefix_stripping_bounds_ciphertext() {
    use crate::aws_kms::AwsKmsError;
    assert!(matches!(
        decode_ciphertext(&"A".repeat(8193)),
        Err(AwsKmsError::ValueTooLarge)
    ));
    assert!(matches!(
        decode_ciphertext(&"\n".repeat(0x0001_0001)),
        Err(AwsKmsError::ValueTooLarge)
    ));
}
