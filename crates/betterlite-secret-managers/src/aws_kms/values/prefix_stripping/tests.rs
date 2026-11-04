#![cfg(test)]

use crate::aws_kms::decode_ciphertext;

#[test]
fn prefix_stripping_removes_only_the_leading_aws_kms_marker() {
    assert_eq!(decode_ciphertext("aws_kms/aGVsbG8=").unwrap(), b"hello");
    assert_eq!(decode_ciphertext("aGVsbG8=").unwrap(), b"hello");
}
