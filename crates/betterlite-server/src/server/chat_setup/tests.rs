#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::{DeploymentError, Provider};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rcgen::{CertificateParams, KeyPair};
use rstest::rstest;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use super::{ChatSetupError, chat_executor};

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"u","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;

fn server_identity() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    let certificate = params.self_signed(&key).unwrap();
    (
        certificate.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
}

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
    let (certificate, private_key) = server_identity();
    let origin = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(origin));
    let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
    let deployment = chat.deployments.first().unwrap();
    let (_served, answered) = tokio::join!(
        serve_tls(&listener, &acceptor),
        chat.send(deployment, &request)
    );
    let observed = answered.map_or_else(|error| error.to_string(), |_outcome| "ok".to_owned());
    assert!(observed.contains(expected), "{observed}");
}

#[rstest]
#[case::a_declared_custom_provider_serves_a_bare_model(
    "      model: vendor-model\n      custom_llm_provider: openai\n      api_key: k\n",
    "vendor-model"
)]
#[case::the_bundled_catalog_resolves_a_bare_openai_model(
    "      model: gpt-4o\n      api_key: k\n",
    "gpt-4o"
)]
fn provider_resolution_tests_behavior(#[case] params: &str, #[case] expected: &str) {
    let yaml = format!("model_list:\n  - model_name: g\n    litellm_params:\n{params}");
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let deployment = chat.deployments.first().unwrap();
    assert_eq!(
        deployment.provider,
        Provider::OpenAi {
            model: expected.to_owned()
        }
    );
}

#[test]
fn a_deployment_no_provider_places_fails_setup_with_the_resolution_error() {
    let yaml =
        "model_list:\n  - model_name: g\n    litellm_params:\n      model: not-a-known-model\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let result = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1));
    assert!(matches!(
        result,
        Err(ChatSetupError::Deployment {
            source: DeploymentError::Resolution { .. }
        })
    ));
}

#[test]
fn a_bedrock_mantle_deployment_still_starts_as_unsupported() {
    let yaml = "model_list:\n  - model_name: m\n    litellm_params:\n      model: bedrock_mantle/openai.gpt-oss-120b\n  - model_name: g\n    litellm_params:\n      model: openai/gpt-4o\n      api_key: k\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let [mantle, openai] = chat.deployments.as_slice() else {
        panic!("both deployments start");
    };
    assert!(matches!(mantle.provider, Provider::Unsupported { .. }));
    assert!(matches!(openai.provider, Provider::OpenAi { .. }));
}

#[test]
fn a_non_openai_deployment_keeps_only_its_configured_credentials() {
    let yaml = "model_list:\n  - model_name: g\n    litellm_params:\n      model: groq/llama\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let environment = BTreeMap::from([("GROQ_API_KEY".to_owned(), "groq-secret".to_owned())]);
    let chat = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    let deployment = chat.deployments.first().unwrap();
    assert!(matches!(deployment.provider, Provider::Unsupported { .. }));
    assert!(deployment.api_key.is_none());
    assert_eq!(deployment.api_base, "https://api.openai.com/v1");
}

#[tokio::test]
async fn a_mocked_network_answers_without_opening_a_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let yaml = format!(
        "litellm_settings:\n  network_mock: true\nmodel_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n      api_key: k\n      api_base: http://127.0.0.1:{port}/v1\n"
    );
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .unwrap();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
    let answered = chat
        .send(chat.deployments.first().unwrap(), &request)
        .await
        .unwrap();
    assert!(answered.response.id.starts_with("chatcmpl-mock-"));
    let connected = timeout(Duration::from_millis(200), listener.accept()).await;
    connected.unwrap_err();
}
