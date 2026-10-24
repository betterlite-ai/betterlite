#![cfg(test)]
use crate::auth::parse_key_header;
use rstest::rstest;
use std::{fs, path::Path};
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
