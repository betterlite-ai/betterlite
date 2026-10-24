#![cfg(test)]
use crate::auth::parse_key_header;
use rstest::rstest;
use secrecy::ExposeSecret as _;
#[rstest]
#[case::aws_empty_credential_before_master(
    "AWS4-HMAC-SHA256 Credential=/x, Credential=master/20261024/eu/service/aws4_request"
)]
fn authorization_prefixes_match_litellm(#[case] value: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), "master");
}
