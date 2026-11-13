use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{limits, secret, settings};
use crate::xai_batches::execution::{CallContext, XaiBatches};

async fn serve_one_batch(listener: TcpListener) -> String {
    let (mut stream, _peer) = listener.accept().await.unwrap();
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await.unwrap();
    let mut header = String::new();
    while reader.read_line(&mut header).await.unwrap() > 2 {
        header.clear();
    }
    let body = r#"{"batch_id":"batch_1"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    request_line
}

#[tokio::test]
async fn the_pooled_client_carries_a_batch_call_over_a_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(serve_one_batch(listener));
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = XaiBatches {
        transport: ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap(),
        settings: settings(),
        limits: limits(),
    };
    let key = secret("key");
    let context = CallContext {
        api_base: Some(&base),
        api_key: Some(&key),
        timeout: Duration::from_secs(5),
    };
    let batch = client.retrieve_batch("batch_1", context).await.unwrap();
    assert_eq!(batch.id, "batch_1");
    let request_line = server.join_next().await.unwrap().unwrap();
    assert_eq!(request_line, "GET /v1/batches/batch_1 HTTP/1.1\r\n");
}
