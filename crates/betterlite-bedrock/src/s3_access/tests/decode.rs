use rstest::rstest;

use crate::s3_access::decode::decode_managed_file_id;
use crate::s3_access::{MANAGED_FILE_ID_ERROR, S3AccessError};

use super::fixtures::{
    INVALID_UTF8_FILE_ID, MANAGED_FILE_ID, MANAGED_OBJECT, MARKER_SEMICOLON_FILE_ID,
    MARKER_TWICE_FILE_ID, NO_MARKER_FILE_ID, ONE_SEXTET_FILE_ID, UNPADDED_FILE_ID, UNPADDED_OBJECT,
};

#[rstest]
#[case::padded_managed_id(MANAGED_FILE_ID, Ok(MANAGED_OBJECT))]
#[case::unpadded_managed_id(UNPADDED_FILE_ID, Ok(UNPADDED_OBJECT))]
#[case::raw_s3_uri(
    "s3://bucket/litellm-bedrock-files-key",
    Ok("s3://bucket/litellm-bedrock-files-key")
)]
#[case::non_ascii_s3_uri("s3://b\u{fc}cket/key", Ok("s3://b\u{fc}cket/key"))]
#[case::marker_twice(MARKER_TWICE_FILE_ID, Ok("first-value"))]
#[case::marker_before_semicolon(MARKER_SEMICOLON_FILE_ID, Ok("value-before"))]
#[case::percent_encoded_scheme("s3%3A%2F%2Fbucket%2Fkey", Err(()))]
#[case::uppercase_scheme("S3://bucket/key", Err(()))]
#[case::plain_text("not-a-file-id", Err(()))]
#[case::non_ascii_text("b\u{fc}", Err(()))]
#[case::prefix_without_marker(NO_MARKER_FILE_ID, Err(()))]
#[case::invalid_utf8(INVALID_UTF8_FILE_ID, Err(()))]
#[case::one_sextet(ONE_SEXTET_FILE_ID, Err(()))]
#[case::truncated("QUJDR", Err(()))]
#[case::empty("", Err(()))]
fn decode_managed_file_id_cases(#[case] file_id: &str, #[case] expected: Result<&str, ()>) {
    assert_eq!(
        decode_managed_file_id(file_id)
            .as_deref()
            .map_err(|_error| ()),
        expected
    );
}

// The ids below are the same managed text without trailing padding: 54 characters (two short) and 55 (one short).
#[rstest]
#[case::two_short(
    "bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iLw",
    Ok("s3://b/")
)]
#[case::one_short(
    "bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s",
    Ok("s3://b/k")
)]
#[case::two_short_with_a_stray_byte("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iLw!", Err(()))]
#[case::one_short_with_a_stray_byte("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s!", Err(()))]
fn padding_is_counted_on_the_raw_length(#[case] file_id: &str, #[case] expected: Result<&str, ()>) {
    assert_eq!(
        decode_managed_file_id(file_id)
            .as_deref()
            .map_err(|_error| ()),
        expected
    );
}

#[test]
fn a_marker_without_a_value_decodes_to_the_empty_string() {
    assert_eq!(
        decode_managed_file_id("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQs")
            .expect("the id decodes"),
        ""
    );
}

#[test]
fn the_rejection_carries_the_litellm_message() {
    let error = decode_managed_file_id("garbage").expect_err("the id cannot decode");
    assert!(matches!(error, S3AccessError::ManagedFileId));
    assert_eq!(error.to_string(), MANAGED_FILE_ID_ERROR);
}

#[test]
fn python_permits_alphabet_bytes_outside_the_url_safe_set() {
    // "litellm_proxy;llm_output_file_id,s3://b/k" in standard base64 has no padding
    assert_eq!(
        decode_managed_file_id("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s=")
            .expect("the standard-alphabet id decodes"),
        "s3://b/k"
    );
    // the URL-safe alphabet substitutes '-' and '_' for the standard '+' and '/'
    assert_eq!(
        decode_managed_file_id("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s74KC-")
            .expect("the dash id decodes"),
        "s3://b/k"
    );
    assert_eq!(
        decode_managed_file_id("bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s74KC_")
            .expect("the underscore id decodes"),
        "s3://b/k"
    );
    // the same value with whitespace and a stray byte, which the permissive decoder drops
    assert_eq!(
        decode_managed_file_id(" bGl0ZWxsbV9wcm94eTtsbG1fb3V0cHV0X2ZpbGVfaWQsczM6Ly9iL2s= ")
            .expect("the whitespace id decodes"),
        "s3://b/k"
    );
}
