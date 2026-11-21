use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::environment_validation::probe::{ModelProbe as _, ProbeCompletion};
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::support::{self, Harness, secret};
use crate::completion_call::probe::{CompletionProbe, ProbeFailure};

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(buffer).unwrap()
}

async fn spawn_server(status: u16) -> (String, JoinSet<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (mut stream, _peer) = listener.accept().await.unwrap();
        let seen = read_request(&mut stream).await;
        let answer = format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\n\r\n");
        stream.write_all(answer.as_bytes()).await.unwrap();
        seen
    });
    (format!("http://{address}/v1"), server)
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

#[tokio::test]
async fn probe_lookup_sends_a_bearer_get_for_the_model() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let (base, mut server) = spawn_server(200).await;
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: Some(base),
        api_key: Some(secret("test-key")),
        timeout: Duration::from_secs(5),
    };
    probe.retrieve_openai_model("gpt/4o").await.unwrap();
    let seen = server.join_next().await.unwrap().unwrap();
    assert!(seen.contains("GET /v1/models/gpt%2F4o HTTP/1.1"), "{seen}");
    assert!(
        seen.to_ascii_lowercase()
            .contains("authorization: bearer test-key"),
        "{seen}"
    );
}

#[tokio::test]
async fn probe_lookup_reports_the_answer_status() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let (base, mut server) = spawn_server(404).await;
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: Some(base),
        api_key: Some(secret("test-key")),
        timeout: Duration::from_secs(5),
    };
    let error = probe.retrieve_openai_model("gpt-4o").await.unwrap_err();
    assert!(matches!(&error, ProbeFailure::Status { status: 404 }));
    server.join_next().await.unwrap().unwrap();
}

#[tokio::test]
async fn probe_lookup_without_a_base_fails() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: None,
        api_key: None,
        timeout: Duration::from_secs(5),
    };
    let error = probe.retrieve_openai_model("gpt-4o").await.unwrap_err();
    assert!(matches!(&error, ProbeFailure::MissingApiBase));
}

#[tokio::test]
async fn probe_complete_calls_the_completion_target() {
    let harness = Harness::new();
    let target = harness.target();
    let client = client();
    let key = secret("probe-key");
    let probe = CompletionProbe {
        client: &client,
        completion: &target,
        api_base: None,
        api_key: None,
        timeout: Duration::from_secs(5),
    };
    let completion = ProbeCompletion {
        model: "gpt-4o",
        user_message: "Hello World",
        api_key: Some(&key),
        max_tokens: Some(10),
    };
    probe.complete(&completion).await.unwrap();
    let seen = harness.chat.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model.as_deref(), Some("gpt-4o"));
    assert_eq!(seen[0].max_tokens, Some(10));
    assert!(seen[0].body.contains(r#""content":"Hello World""#));
    let lookups = support::lock(&harness.metadata.lookups).clone();
    assert_eq!(lookups.len(), 1);
    assert_eq!(lookups[0].api_key.as_deref(), Some("probe-key"));
}
