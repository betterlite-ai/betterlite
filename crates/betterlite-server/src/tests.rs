#![cfg(test)]
use crate::auth::parse_key_header;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::fs;
use std::path::Path;
#[rstest]
#[case::lowercase_bearer("bearer master")]
#[case::basic("Basic master")]
#[case::repeated_bearer("Bearer Bearer master")]
#[case::aws("AWS4-HMAC-SHA256 Credential=Bearer master/20261024/eu/service/aws4_request")]
#[case::aws_several_spaces(
    "AWS4-HMAC-SHA256 Credential=Bearer   master/20261024/eu/service/aws4_request"
)]
#[case::aws_tab("AWS4-HMAC-SHA256 Credential=Bearer\tmaster/20261024/eu/service/aws4_request")]
#[case::aws_bearer_without_whitespace(
    "AWS4-HMAC-SHA256 Credential=Bearerabc/20261024/eu/service/aws4_request"
)]
#[case::aws_bearer_without_token(
    "AWS4-HMAC-SHA256 Credential=Bearer/20261024/eu/service/aws4_request"
)]
fn authorization_prefixes_match_litellm(#[case] value: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    let expected = if value.contains("Bearerabc/") {
        "Bearerabc"
    } else if value.contains("Credential=Bearer/") {
        "Bearer"
    } else {
        "master"
    };
    assert_eq!(key.expose_secret(), expected);
}
#[rstest]
#[case::authorization("authorization")]
#[case::api_key("api-key")]
#[case::x_api_key("x-api-key")]
#[case::x_goog_api_key("x-goog-api-key")]
#[case::ocp_apim("ocp-apim-subscription-key")]
#[case::x_litellm_api_key("x-litellm-api-key")]
fn replays_every_key_header_fuzz_corpus_file(#[case] name: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/key_header");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .for_each(|path| {
            let input = fs::read(path).unwrap();
            let _key = parse_key_header(name, &input);
        });
}
