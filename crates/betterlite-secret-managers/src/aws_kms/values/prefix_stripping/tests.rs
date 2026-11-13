#![cfg(test)]

use crate::aws_kms::decode_ciphertext;
use rstest::rstest;

#[rstest]
#[case::plain("aGVsbG8=")]
#[case::prefixed("aws_kms/aGVsbG8=")]
#[case::newline("aws_kms/aGVsbG8=\n")]
#[case::wrapped("aGVs\r\nbG8=\n")]
#[case::ignored_punctuation("aG!Vs bG8=")]
#[case::every_marker_after_the_prefix("aws_kms/aGVsaws_kms/bG8=")]
fn prefix_stripping_accepts_python_base64(#[case] input: &str) {
    assert_eq!(decode_ciphertext(input).unwrap(), b"hello");
}

#[test]
fn prefix_stripping_keeps_a_marker_when_the_value_does_not_start_with_it() {
    assert_eq!(
        decode_ciphertext("YWFhaws_kms/=").unwrap(),
        [97, 97, 97, 107, 11, 36, 154, 207]
    );
}

#[rstest]
#[case::excess_padding("aGk==", &[104, 105])]
#[case::leading_padding("=aGk=", &[104, 105])]
#[case::trailing_bits("aGl=", &[104, 105])]
#[case::junk_between_pads("aG=!=", &[104])]
#[case::data_after_padding("aGk=x", &[104, 105, 49])]
#[case::data_between_pads("aG=a=", &[104, 102])]
#[case::padding_inside_last_quad("aG=k=", &[104, 105])]
fn prefix_stripping_follows_python_lenient_padding(#[case] input: &str, #[case] expected: &[u8]) {
    assert_eq!(decode_ciphertext(input).unwrap(), expected);
}

#[rstest]
#[case::missing_padding("aGk", "padding", 0)]
#[case::data_inside_padding("aG=k", "padding", 0)]
#[case::padding_does_not_end_input("aGVsbG8=aGk=", "padding", 0)]
#[case::dangling_symbol("a===", "length", 1)]
#[case::dangling_symbol_after_padding("aG==aGk=", "length", 5)]
fn prefix_stripping_rejects_what_python_rejects(
    #[case] input: &str,
    #[case] expected: &str,
    #[case] characters: usize,
) {
    use crate::aws_kms::AwsKmsError;
    let error = decode_ciphertext(input).err().unwrap();
    match expected {
        "padding" => assert!(matches!(error, AwsKmsError::Base64Padding)),
        "length" => {
            assert!(matches!(error, AwsKmsError::Base64Length(count) if count == characters));
        }
        _ => panic!("unknown case"),
    }
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
