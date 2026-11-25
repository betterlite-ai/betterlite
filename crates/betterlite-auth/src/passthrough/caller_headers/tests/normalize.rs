#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::credential_value::normalize_credential_value;
use super::support::secret;

#[rstest]
#[case::bearer("Bearer sk-x", "sk-x")]
#[case::lowercase_bearer("bearer sk-x", "sk-x")]
#[case::basic("Basic dXNlcjpwYXNz", "dXNlcjpwYXNz")]
#[case::uppercase_bearer_is_unrecognized("BEARER sk-x", "BEARER sk-x")]
#[case::bearer_with_a_tab_is_unrecognized("Bearer\tsk-x", "Bearer\tsk-x")]
#[case::bearer_removes_every_occurrence("Bearer aBearer b", "ab")]
#[case::bearer_without_a_token("Bearer ", "Bearer ")]
#[case::basic_without_a_token("Basic ", "Basic ")]
#[case::plain_token("sk-plain", "sk-plain")]
#[case::empty_value("", "")]
#[case::aws_bearer_credential(
    "AWS4-HMAC-SHA256 Credential=Bearer sk-9/20250101/us-east-1/bedrock/aws4_request, SignedHeaders=host, Signature=abc",
    "sk-9"
)]
#[case::aws_plain_credential(
    "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20250101/us-east-1/service/aws4_request",
    "AKIDEXAMPLE"
)]
#[case::aws_bearer_credential_without_whitespace("AWS4-HMAC-SHA256 Credential=Bearer/x", "Bearer")]
#[case::aws_without_a_credential(
    "AWS4-HMAC-SHA256 SignedHeaders=host, Signature=abc",
    "AWS4-HMAC-SHA256 SignedHeaders=host, Signature=abc"
)]
fn a_recognized_scheme_is_stripped_and_anything_else_keeps_the_value(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let normalized = normalize_credential_value(&secret(value));
    assert!(normalized.expose_secret() == expected);
}
