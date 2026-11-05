#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::ChatRequest;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::chat_executor;

const SERVER_CERT: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.pem");
const SERVER_KEY: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.key");
const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"u","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;

async fn serve_tls(listener: &TcpListener, acceptor: &TlsAcceptor) {
    let (socket, _) = listener.accept().await.unwrap();
    let Ok(mut stream) = acceptor.accept(socket).await else {
        return;
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ANSWER}",
        ANSWER.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
}

#[rstest]
#[case::a_deployment_ssl_verify_reaches_its_client("      ssl_verify: false\n", "ok")]
#[case::a_deployment_without_ssl_verify_keeps_verification("", "could not be reached")]
#[tokio::test]
async fn http_pool_tests_behavior(#[case] verify: &str, #[case] expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let yaml = format!(
        "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n      api_key: k\n      api_base: https://127.0.0.1:{port}/v1\n{verify}"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let certificates = CertificateDer::pem_slice_iter(SERVER_CERT)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let origin = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            certificates,
            PrivateKeyDer::from_pem_slice(SERVER_KEY).unwrap(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(origin));
    let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
    let deployment = chat.pick("g").unwrap();
    let (_served, answered) = tokio::join!(
        serve_tls(&listener, &acceptor),
        chat.send(deployment, &request)
    );
    let observed = answered.map_or_else(|error| error.to_string(), |_outcome| "ok".to_owned());
    assert!(observed.contains(expected), "{observed}");
}
