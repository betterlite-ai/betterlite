#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::fake::{batch, create_request, query};
use crate::batches::error::SdkError;
use crate::batches::http::{HttpBatchesClient, HttpConstructor};
use crate::batches::records::BatchObject;
use crate::batches::sdk::{
    BatchesSdk as _, ClientConstructor as _, ClientKind, SdkArguments, Timeout, TimeoutPhases,
};

const SECRET: &str = "sk-fixture-secret-7731";

fn constructor(environment_key: Option<&str>) -> HttpConstructor {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpConstructor {
        transport: ProxiedClient::outbound(&policy).unwrap(),
        environment_key: environment_key.map(SecretString::from),
    }
}

fn arguments(api_key: Option<&str>, base_url: &str, organization: Option<&str>) -> SdkArguments {
    SdkArguments {
        api_key: api_key.map(SecretString::from),
        base_url: Some(base_url.to_owned()),
        timeout: None,
        max_retries: None,
        organization: organization.map(str::to_owned),
    }
}

fn arguments_with_timeout(timeout: Option<Timeout>) -> SdkArguments {
    SdkArguments {
        api_key: Some(SecretString::from("k")),
        base_url: Some(String::from("https://example.test/v1")),
        timeout,
        max_retries: None,
        organization: None,
    }
}

fn build(
    builder: &HttpConstructor,
    arguments: &SdkArguments,
    kind: ClientKind,
) -> HttpBatchesClient {
    let client = builder.construct(arguments, kind).unwrap();
    client.unwrap()
}

async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    (listener, base)
}

async fn read_request(stream: &mut TcpStream) -> String {
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
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

async fn serve(listener: TcpListener, replies: Vec<(u16, String)>) -> Vec<String> {
    let mut requests = Vec::new();
    for (status, body) in replies {
        let (mut stream, _peer) = listener.accept().await.unwrap();
        requests.push(read_request(&mut stream).await);
        let reply = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
    }
    requests
}

fn body_of(request: &str) -> Value {
    serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

fn pair(name: &str, value: &str) -> IndexMap<String, String> {
    IndexMap::from([(String::from(name), String::from(value))])
}

fn assert_batch_requests(requests: &[String]) {
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("POST /v1/batches HTTP/1.1"));
    assert!(requests[0].contains("x-trace: t1"));
    let expected = r#"{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1","vendor":"x"}"#;
    assert_eq!(
        body_of(&requests[0]),
        serde_json::from_str::<Value>(expected).unwrap()
    );
    assert!(requests[1].starts_with("GET /v1/batches/batch_1 HTTP/1.1"));
    assert!(requests[2].starts_with("POST /v1/batches/batch_1/cancel HTTP/1.1"));
    assert!(requests[3].starts_with("GET /v1/batches HTTP/1.1"));
    assert!(requests[4].starts_with("GET /v1/batches?after=batch_0&limit=2 HTTP/1.1"));
}

#[tokio::test]
async fn http_client_speaks_the_batch_endpoints() {
    let ok = batch("batch_1", "validating");
    let page = format!(
        r#"{{"data":[{}],"has_more":true}}"#,
        batch("batch_1", "completed")
    );
    let replies = vec![
        (200, ok.clone()),
        (200, ok.clone()),
        (200, ok),
        (200, page.clone()),
        (200, page),
    ];
    let (listener, base) = bind().await;
    let client = build(
        &constructor(None),
        &arguments(Some("k"), &base, None),
        ClientKind::Async,
    );
    let mut request = create_request();
    request.extra_body = Some(pair("vendor", "x"));
    request.extra_headers = Some(pair("x-trace", "t1"));
    let calls = async {
        let bytes = client.create(&request).await.unwrap();
        let created: BatchObject = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created.id, "batch_1");
        client.retrieve(&query()).await.unwrap();
        client.cancel(&query()).await.unwrap();
        assert_eq!(client.list(None, None).await.unwrap().has_more, Some(true));
        client.list(Some("batch_0"), Some(2)).await.unwrap();
    };
    let (requests, ()) = tokio::join!(serve(listener, replies), calls);
    assert_batch_requests(&requests);
}

#[tokio::test]
async fn http_client_reports_status_errors_with_their_fields() {
    let error = String::from(
        r#"{"error":{"message":"no","type":"invalid_request_error","param":"limit","code":"bad"}}"#,
    );
    let (listener, base) = bind().await;
    let client = build(
        &constructor(None),
        &arguments(Some("k"), &base, None),
        ClientKind::Async,
    );
    let (_requests, failure) = tokio::join!(
        serve(listener, vec![(400, error)]),
        client.list(None, Some(0))
    );
    let SdkError::Status {
        status,
        error_type,
        param,
        code,
        message,
    } = failure.unwrap_err()
    else {
        panic!("expected a status error");
    };
    assert_eq!(status, 400);
    assert_eq!(error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(param.as_deref(), Some("limit"));
    assert_eq!(code.as_deref(), Some("bad"));
    assert!(message.starts_with("Error code: 400 - "), "{message}");
}

#[tokio::test]
async fn openai_batches_credential_identity_isolation() {
    let replies = vec![(200, batch("b", "validating")); 3];
    let (listener, base) = bind().await;
    let environment = constructor(Some("sk-from-environment"));
    let explicit = build(
        &environment,
        &arguments(Some("sk-explicit"), &base, Some("org-a")),
        ClientKind::Sync,
    );
    let inherited = build(
        &environment,
        &arguments(None, &base, None),
        ClientKind::Sync,
    );
    let empty_key = build(
        &environment,
        &arguments(Some(""), &base, None),
        ClientKind::Sync,
    );
    let calls = async {
        explicit.retrieve(&query()).await.unwrap();
        inherited.retrieve(&query()).await.unwrap();
        empty_key.retrieve(&query()).await.unwrap();
    };
    let (requests, ()) = tokio::join!(serve(listener, replies), calls);
    assert!(requests[0].contains("authorization: Bearer sk-explicit\r\n"));
    assert!(requests[0].contains("openai-organization: org-a\r\n"));
    assert!(requests[1].contains("authorization: Bearer sk-from-environment\r\n"));
    assert!(!requests[1].contains("openai-organization"));
    assert!(!requests[1].contains("sk-explicit"));
    assert!(!requests[2].contains("authorization"));
    assert!(!requests[2].contains("sk-from-environment"));
    let without_key = constructor(None).construct(&arguments(None, &base, None), ClientKind::Sync);
    assert!(matches!(without_key, Ok(None)));
}

#[tokio::test]
async fn openai_batches_secret_safe_failure() {
    let (unreachable, dead) = bind().await;
    drop(unreachable);
    let builder = constructor(None);
    let client = build(
        &builder,
        &arguments(Some(SECRET), &dead, Some("org-x")),
        ClientKind::Sync,
    );
    let broken = build(
        &builder,
        &arguments(Some("sk-bad\nkey"), &dead, None),
        ClientKind::Sync,
    );
    let failures = [
        client.retrieve(&query()).await.unwrap_err(),
        broken.retrieve(&query()).await.unwrap_err(),
    ];
    let shown = format!(
        "{client:?} {broken:?} {:?} {:?} {:?}",
        arguments(Some(SECRET), &dead, None),
        failures.each_ref().map(|error| format!("{error}")),
        failures.each_ref().map(|error| format!("{error:?}")),
    );
    assert!(!shown.contains(SECRET), "{shown}");
    assert!(!shown.contains("sk-bad"), "{shown}");
    assert!(matches!(failures[1], SdkError::Transport { .. }));
}

#[test]
fn batch_client_timeouts_reject_unusable_numbers() {
    let builder = constructor(None);
    let errors = [-1.0_f64, f64::NAN, f64::INFINITY, f64::MAX].map(|seconds| {
        builder
            .construct(
                &arguments_with_timeout(Some(Timeout::Seconds(seconds))),
                ClientKind::Sync,
            )
            .unwrap_err()
    });
    assert!(
        errors
            .iter()
            .all(|error| matches!(error, SdkError::InvalidTimeout { .. }))
    );
    let zero = build(
        &builder,
        &arguments_with_timeout(Some(Timeout::Seconds(0.0_f64))),
        ClientKind::Sync,
    );
    assert_eq!(zero.timeout, Duration::ZERO);
}

#[test]
fn batch_client_timeouts_name_unsupported_phases() {
    let timeout = Timeout::Phases(TimeoutPhases {
        connect: Some(1.0_f64),
        read: None,
        write: None,
        pool: Some(2.0_f64),
    });
    let error = constructor(None)
        .construct(&arguments_with_timeout(Some(timeout)), ClientKind::Sync)
        .unwrap_err();
    let SdkError::UnsupportedTimeoutPhase { phases } = error else {
        panic!("expected the phases to be refused");
    };
    assert!(phases.contains("connect"), "{phases}");
    assert!(phases.contains("pool"), "{phases}");
    assert!(!phases.contains("read"), "{phases}");
}

#[tokio::test]
async fn batch_calls_reject_an_unusable_timeout() {
    let (listener, base) = bind().await;
    drop(listener);
    let client = build(
        &constructor(None),
        &arguments(Some("k"), &base, None),
        ClientKind::Sync,
    );
    let mut not_a_number = query();
    not_a_number.timeout = Some(f64::NAN);
    let mut negative = query();
    negative.timeout = Some(-1.0_f64);
    let failures = [
        client.retrieve(&not_a_number).await.unwrap_err(),
        client.retrieve(&negative).await.unwrap_err(),
    ];
    assert!(
        failures
            .iter()
            .all(|error| matches!(error, SdkError::InvalidTimeout { .. }))
    );
}
