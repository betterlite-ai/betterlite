#![cfg(test)]
use crate::auth::parse_key_header;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::fs;
use std::path::Path;
#[rstest]
#[case::lowercase_bearer("bearer master")]
#[case::basic("Basic master")]
#[case::aws("AWS4-HMAC-SHA256 Credential=Bearer master/20261024/eu/service/aws4_request")]
#[case::aws_several_spaces(
    "AWS4-HMAC-SHA256 Credential=Bearer   master/20261024/eu/service/aws4_request"
)]
#[case::aws_tab("AWS4-HMAC-SHA256 Credential=Bearer\tmaster/20261024/eu/service/aws4_request")]
fn authorization_prefixes_match_litellm(#[case] value: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), "master");
}
#[rstest]
#[case::authorization("authorization")]
#[case::api_key("api-key")]
#[case::x_api_key("x-api-key")]
#[case::x_goog_api_key("x-goog-api-key")]
#[case::ocp_apim("ocp-apim-subscription-key")]
#[case::x_litellm_api_key("x-litellm-api-key")]
fn every_empty_key_header_is_absent(#[case] name: &str) {
    assert!(parse_key_header(name, b"").is_none());
}
#[test]
fn replays_every_key_header_fuzz_corpus_file() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/key_header");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .for_each(|path| {
            let input = fs::read(path).unwrap();
            let _key = parse_key_header("authorization", &input);
        });
}
