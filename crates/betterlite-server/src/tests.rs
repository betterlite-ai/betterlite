#![cfg(test)]
use crate::auth::parse_key_header;
use crate::passthrough::{EndpointType, get_endpoint_type};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::{fs, path::Path};

#[test]
fn passthrough_endpoint_type_behavior() {
    assert_eq!(
        get_endpoint_type("https://vertex.example/generateContent", None),
        EndpointType::VertexAi
    );
    assert_eq!(
        get_endpoint_type("https://api.openai.com/v1/chat", None),
        EndpointType::OpenAi
    );
}

#[test]
fn passthrough_payload() {}
#[rstest]
#[case::repeated_bearer("Bearer test-Bearer master", "test-master")]
#[case::repeated_basic("Basic test-Basic master", "test-master")]
#[case::repeated_lowercase("bearer test-bearer master", "test-master")]
#[case::aws_empty_credential_before_master(
    "AWS4-HMAC-SHA256 Credential=/x, Credential=master/20261024/eu/service/aws4_request",
    "master"
)]
#[case::aws_first_credential(
    "AWS4-HMAC-SHA256 Credential=first, Credential=Bearer master/20261024/eu/service/aws4_request",
    "master"
)]
#[case::aws_first_bearer(
    "AWS4-HMAC-SHA256 Credential=Bearer test-master/d/r/s/aws4_request, Credential=other/d",
    "test-master"
)]
#[case::aws_bearer_before_plain(
    "AWS4-HMAC-SHA256 Credential=other/d, Credential=Bearer test-master/d",
    "test-master"
)]
#[case::aws_first_plain(
    "AWS4-HMAC-SHA256 Credential=test-master/d, Credential=other/d",
    "test-master"
)]
#[case::aws_two_bearers(
    "AWS4-HMAC-SHA256 Credential=Bearer test-master/d, Credential=Bearer other/d",
    "test-master"
)]
#[case::aws_empty("AWS4-HMAC-SHA256 Credential=/x", "")]
fn authorization_prefixes_match_litellm(#[case] value: &str, #[case] expected: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), expected);
}
#[rstest]
#[case::invalid_byte(b"invalid-\xFF")]
#[case::utf8_bytes(b"invalid-\xC3\xBF")]
fn non_utf8_header_is_used_as_wrong_key(#[case] value: &[u8]) {
    let key = parse_key_header("x-litellm-api-key", value).unwrap();
    assert_ne!(key.expose_secret(), "test-master");
}
#[rstest]
#[case::authorization("authorization")]
#[case::api_key("api-key")]
#[case::x_api_key("x-api-key")]
#[case::x_goog_api_key("x-goog-api-key")]
#[case::ocp_apim_subscription_key("ocp-apim-subscription-key")]
#[case::x_litellm_api_key("x-litellm-api-key")]
fn replays_every_key_header_fuzz_corpus_file(#[case] name: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/key_header");
    for entry in fs::read_dir(corpus).unwrap() {
        let input = fs::read(entry.unwrap().path()).unwrap();
        let _key = parse_key_header(name, &input);
    }
}
