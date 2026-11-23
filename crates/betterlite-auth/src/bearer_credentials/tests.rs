#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;

use super::get_bearer_token;

#[rstest]
#[case::bearer_scheme("Bearer sk-123", Some("sk-123"), "sk-123")]
#[case::bearer_removes_every_occurrence("Bearer aBearer b", Some("ab"), "ab")]
#[case::bearer_removes_a_repeated_prefix("Bearer Bearer sk-123", Some("sk-123"), "sk-123")]
#[case::bearer_keeps_a_second_space("Bearer  sk-123", Some(" sk-123"), " sk-123")]
#[case::bearer_without_a_key("Bearer ", Some(""), "")]
#[case::bearer_leaves_a_basic_prefix(
    "Bearer Basic dXNlcjpwYXNz",
    Some("Basic dXNlcjpwYXNz"),
    "Basic dXNlcjpwYXNz"
)]
#[case::basic_scheme("Basic dXNlcjpwYXNz", Some("dXNlcjpwYXNz"), "dXNlcjpwYXNz")]
#[case::basic_removes_every_occurrence(
    "Basic Basic dXNlcjpwYXNz",
    Some("dXNlcjpwYXNz"),
    "dXNlcjpwYXNz"
)]
#[case::basic_without_a_payload("Basic ", Some(""), "")]
#[case::lowercase_bearer("bearer sk-123", Some("sk-123"), "sk-123")]
#[case::lowercase_bearer_removes_every_occurrence("bearer bearer sk-123", Some("sk-123"), "sk-123")]
#[case::lowercase_bearer_leaves_the_uppercase_prefix(
    "bearer Bearer sk-123",
    Some("Bearer sk-123"),
    "Bearer sk-123"
)]
#[case::uppercase_bearer_is_unrecognized("BEARER sk-123", None, "BEARER sk-123")]
#[case::tab_after_bearer_is_unrecognized("Bearer\tsk-123", None, "Bearer\tsk-123")]
#[case::bearer_without_a_space("Bearersk-123", None, "Bearersk-123")]
#[case::uppercase_basic_is_unrecognized("BASIC dXNlcjpwYXNz", None, "BASIC dXNlcjpwYXNz")]
#[case::plain_key("sk-123", None, "sk-123")]
#[case::empty_value("", None, "")]
#[case::aws_bearer_credential(
    "AWS4-HMAC-SHA256 Credential=Bearer sk-12345/20250101/us-east-1/bedrock/aws4_request, SignedHeaders=host, Signature=abc",
    Some("sk-12345"),
    "sk-12345"
)]
#[case::aws_without_a_space_before_the_credential("AWS4-HMAC-SHA256Credential=k/x", Some("k"), "k")]
#[case::aws_bearer_without_whitespace_after_it(
    "AWS4-HMAC-SHA256 Credential=Bearer/x",
    Some("Bearer"),
    "Bearer"
)]
#[case::aws_plain_credential(
    "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20250101/us-east-1/service/aws4_request",
    Some("AKIDEXAMPLE"),
    "AKIDEXAMPLE"
)]
#[case::aws_prefers_the_bearer_credential(
    "AWS4-HMAC-SHA256 Credential=plain, Credential=Bearer sk-9/x",
    Some("sk-9"),
    "sk-9"
)]
#[case::aws_first_credential_wins(
    "AWS4-HMAC-SHA256 Credential=one, Credential=two",
    Some("one"),
    "one"
)]
#[case::aws_without_a_credential(
    "AWS4-HMAC-SHA256 SignedHeaders=host, Signature=abc",
    None,
    "AWS4-HMAC-SHA256 SignedHeaders=host, Signature=abc"
)]
#[case::aws_with_an_empty_credential(
    "AWS4-HMAC-SHA256 Credential=/20250101",
    None,
    "AWS4-HMAC-SHA256 Credential=/20250101"
)]
#[case::aws_credential_after_a_file_separator(
    "AWS4-HMAC-SHA256 Credential=Bearer\u{1c}sk-7/20250101",
    Some("sk-7"),
    "sk-7"
)]
#[case::aws_file_separator_ends_the_token(
    "AWS4-HMAC-SHA256 Credential=sk-7\u{1e}rest",
    Some("sk-7"),
    "sk-7"
)]
#[case::aws_non_breaking_space_ends_the_token(
    "AWS4-HMAC-SHA256 Credential=sk-7\u{a0}rest",
    Some("sk-7"),
    "sk-7"
)]
#[case::credential_without_the_aws_prefix(
    "Signature=abc Credential=Bearer sk-1/x",
    None,
    "Signature=abc Credential=Bearer sk-1/x"
)]
fn parses_the_presented_key(
    #[case] value: &str,
    #[case] strict: Option<&str>,
    #[case] permissive: &str,
) {
    let token = get_bearer_token(value);
    let strict_result = token.as_ref().map(ExposeSecret::expose_secret);
    assert_eq!(strict_result, strict);
    assert_eq!(strict_result.unwrap_or(value), permissive);
}
