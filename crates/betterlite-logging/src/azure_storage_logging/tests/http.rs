#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::Method;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::join;
use tokio::net::TcpListener;

use super::support::{ENTRA, Reply, fixture, payload};
use crate::azure_storage_logging::transport::{
    PooledTransport, StorageRequest, StorageTransport as _,
};
use crate::fields::payload::StandardLoggingPayload;

#[tokio::test]
async fn the_pooled_transport_sends_the_request_and_reads_the_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let served = async move {
        let (mut stream, _peer) = listener.accept().await.expect("one request");
        let mut received = Vec::new();
        while !received.ends_with(b"\r\n\r\n{}") {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await.expect("a request read");
            assert!(read > 0, "the client closed before the body");
            received.extend_from_slice(&chunk[..read]);
        }
        let answer = b"HTTP/1.1 201 Created\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
        stream.write_all(answer).await.expect("the answer write");
        String::from_utf8(received).expect("an ASCII request")
    };
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    let transport = PooledTransport {
        client: ProxiedClient::outbound(&policy).expect("http client"),
    };
    let request = StorageRequest {
        method: Method::PATCH,
        url: format!("http://{address}/logs/id-1.json?action=append&position=0"),
        query: Vec::new(),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("x-ms-version".to_owned(), "2019-07-07".to_owned()),
        ],
        body: b"{}".to_vec(),
        deadline: Duration::from_secs(5),
    };
    let (sent, received) = join!(transport.send(request), served);
    let response = sent.expect("the local server answers");
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"ok");
    assert!(received.starts_with("PATCH /logs/id-1.json?action=append&position=0 HTTP/1.1\r\n"));
    let head = received.to_ascii_lowercase();
    assert!(head.contains("content-type: application/json\r\n"));
    assert!(head.contains("x-ms-version: 2019-07-07\r\n"));
}

#[tokio::test]
async fn the_token_path_creates_appends_and_flushes_one_file() {
    let fixture = fixture(&ENTRA, &[Reply::TOKEN, Reply::OK, Reply::OK, Reply::OK]);
    let payload = payload("resp_YWJj/ZA==");
    fixture
        .logger
        .upload_payload(&payload)
        .await
        .expect("the scripted storage accepts the upload");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 4);
    let create = calls.get(1).expect("the create call");
    assert_eq!(create.method, "PUT");
    assert_eq!(
        create.url,
        "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?resource=file"
    );
    assert_eq!(create.header("x-ms-version"), Some("2019-07-07"));
    assert_eq!(create.header("Content-Length"), Some("0"));
    assert_eq!(create.header("Content-Type"), None);
    assert_eq!(create.header("Authorization"), Some("Bearer tok-1"));
    let append = calls.get(2).expect("the append call");
    assert_eq!(append.method, "PATCH");
    assert_eq!(
        append.url,
        "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?action=append&position=0"
    );
    assert_eq!(append.header("Content-Type"), Some("application/json"));
    assert_eq!(append.header("Content-Length"), None);
    assert_eq!(append.header("Authorization"), Some("Bearer tok-1"));
    let text = from_utf8(&append.body).expect("utf-8 json");
    assert!(text.ends_with('\n'));
    let body: StandardLoggingPayload =
        serde_json::from_str(text.trim_end_matches('\n')).expect("json body");
    assert_eq!(body.id, "resp_YWJj/ZA==");
    let flush = calls.get(3).expect("the flush call");
    assert_eq!(flush.method, "PATCH");
    assert_eq!(
        flush.url,
        format!(
            "https://acct.dfs.core.windows.net/logs/resp_YWJj_ZA.json?action=flush&position={}",
            append.body.len()
        )
    );
    assert_eq!(flush.header("Content-Length"), Some("0"));
}

#[tokio::test]
async fn a_failed_create_stops_before_the_append() {
    let fixture = fixture(
        &ENTRA,
        &[
            Reply::TOKEN,
            Reply {
                status: 500,
                body: "",
            },
        ],
    );
    let payload = payload("id-1");
    let error = fixture
        .logger
        .upload_payload(&payload)
        .await
        .expect_err("a 500 is an upload failure");
    assert!(error.to_string().contains("500"));
    assert_eq!(fixture.transport.calls().len(), 2);
}

#[tokio::test]
async fn each_upload_writes_its_own_file() {
    let fixture = fixture(
        &ENTRA,
        &[
            Reply::TOKEN,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
        ],
    );
    fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect("first upload");
    fixture
        .logger
        .upload_payload(&payload("id-2"))
        .await
        .expect("second upload");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 7);
    assert!(calls.get(1).expect("create").url.contains("/id-1.json?"));
    assert!(calls.get(4).expect("create").url.contains("/id-2.json?"));
}
