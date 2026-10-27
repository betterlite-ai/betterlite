#![cfg(test)]
use crate::auth::parse_key_header;
use crate::server::chat_setup::chat_executor;
use crate::server::run;
use betterlite_config::loader::ConfigLoader;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::time::Duration;
use std::{fs, path::Path};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
#[tokio::test]
async fn run_serves_requests_with_the_chat_executor_it_was_given() {
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let environment = BTreeMap::new();
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader
        .load("model_list:\n  - {model_name: g, litellm_params: {model: azure/m}}\n")
        .unwrap();
    let chat = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    let (served, response) = tokio::join!(
        timeout(Duration::from_secs(3), run(config, chat, "127.0.0.1", port)),
        timeout(Duration::from_secs(3), post_chat_once_listening(port))
    );
    assert!(
        served.is_err(),
        "run stopped while it should serve: {served:?}"
    );
    let answer = response.unwrap();
    assert!(answer.starts_with("HTTP/1.1 501"), "{answer}");
}
async fn post_chat_once_listening(port: u16) -> String {
    let mut connected = TcpStream::connect(("127.0.0.1", port)).await;
    while connected.is_err() {
        sleep(Duration::from_millis(10)).await;
        connected = TcpStream::connect(("127.0.0.1", port)).await;
    }
    let mut stream = connected.unwrap();
    let body = r#"{"model":"g","messages":[]}"#;
    let request = format!(
        "POST /chat/completions HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}
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
