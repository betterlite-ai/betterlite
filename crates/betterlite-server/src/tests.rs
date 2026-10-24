#![cfg(test)]
use crate::auth::parse_key_header;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::{fs, path::Path};
#[rstest]
#[case::aws_bearer_after_another_credential(
    "AWS4-HMAC-SHA256 Credential=first, Credential=Bearer master/20261024/eu/service/aws4_request"
)]
fn authorization_prefixes_match_litellm(#[case] value: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), "master");
}
#[rstest]
#[case::authorization("authorization")]
fn replays_every_key_header_fuzz_corpus_file(#[case] name: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/key_header");
    for entry in fs::read_dir(corpus).unwrap() {
        let input = fs::read(entry.unwrap().path()).unwrap();
        let _key = parse_key_header(name, &input);
    }
}
