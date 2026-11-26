#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::auth::parse_key_header;

// the scheme each header name strips: authorization takes every prefix, x-litellm-api-key only the schemes, the other key headers stay as sent
#[rstest]
#[case::authorization_bearer("authorization", "Bearer test-master", "test-master")]
#[case::authorization_basic("authorization", "Basic test-Basic master", "test-master")]
#[case::litellm_key_bearer("x-litellm-api-key", "Bearer sk-1", "sk-1")]
#[case::litellm_key_plain("x-litellm-api-key", "sk-1", "sk-1")]
#[case::plain_header_kept("api-key", "Bearer sk-1", "Bearer sk-1")]
fn presented_key_strips_each_header_scheme(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let key = parse_key_header(name, value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), expected);
}

// the SigV4 credential inside an AWS authorization header, with its / scope dropped
#[rstest]
#[case::plain(
    "AWS4-HMAC-SHA256 Credential=test-master/d/r/s/aws4_request",
    "test-master"
)]
#[case::bearer(
    "AWS4-HMAC-SHA256 Credential=Bearer test-master/d, Credential=other/d",
    "test-master"
)]
#[case::empty("AWS4-HMAC-SHA256 Credential=/x", "")]
fn aws_authorization_yields_its_credential(#[case] value: &str, #[case] expected: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), expected);
}

#[test]
fn empty_header_bytes_yield_none() {
    assert!(parse_key_header("authorization", b"").is_none());
}
