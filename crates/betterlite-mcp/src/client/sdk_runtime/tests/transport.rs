use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::sync::Arc;

use bytes::Bytes;
use http::Method;
use http::header::HeaderMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::client::cleanup::http_client::CleanupScope;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::factory::{AuthSource, HttpClientConfig, TlsSettings};
use crate::client::sessions::runtime::{HttpClientHandle as _, McpRuntime as _};
use crate::client::sessions::transport::TransportType;
use crate::client::tools::listing::list_tools;

use super::support;

#[tokio::test]
async fn an_unparsable_url_leaves_a_transport_that_cannot_open() {
    let streamable_http = unavailable_reason(TransportType::Http).await;
    assert!(
        streamable_http.contains("streamable HTTP"),
        "{streamable_http}"
    );
    let legacy_sse = unavailable_reason(TransportType::Sse).await;
    assert!(legacy_sse.contains("legacy SSE"), "{legacy_sse}");
}

async fn unavailable_reason(transport: TransportType) -> String {
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(transport),
        "not a url",
        support::CALL_TIMEOUT,
    );
    let error = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .expect_err("the transport cannot open");
    let SessionError::Runtime(source) = error else {
        panic!("expected a runtime failure");
    };
    let SdkError::Io(inner) = source.as_ref() else {
        panic!("expected the unavailable stdio transport");
    };
    assert_eq!(inner.kind(), ErrorKind::Unsupported);
    inner.to_string()
}

#[tokio::test]
async fn a_cleanup_scope_carries_the_body_failure_the_client_recorded() {
    let url = serve_truncated_stream().await;
    let runtime = support::runtime();
    let config = HttpClientConfig {
        headers: BTreeMap::new(),
        timeout: support::CALL_TIMEOUT,
        tls: TlsSettings::from_policy(&support::tls()),
        auth: AuthSource::None,
        credential: None,
        follow_redirects: true,
        redirect_guard: None,
        capture_error_responses: true,
    };
    let client = runtime.http_client(config).unwrap();
    client.begin_cleanup(cleanup_scope());
    assert!(client.finish_cleanup().is_empty());
    client.begin_cleanup(cleanup_scope());
    let request = McpHttpRequest {
        method: Method::POST,
        url: Url::parse(&url).unwrap(),
        headers: HeaderMap::new(),
        body: Bytes::new(),
        read_timeout: None,
    };
    let mut response = client.client.send(request, true).await.unwrap();
    let first = response.body.next_chunk().await.unwrap().unwrap();
    assert!(!first.is_empty());
    let failure = response.body.next_chunk().await.unwrap().unwrap_err();
    let mut cleanup = client.finish_cleanup();
    let error = cleanup.remove(0);
    let SdkError::Http(kept) = error.as_ref() else {
        panic!("expected the recorded HTTP failure");
    };
    assert!(Arc::ptr_eq(kept, &failure));
    assert!(cleanup.is_empty());
    assert!(client.finish_cleanup().is_empty());
}

fn cleanup_scope() -> CleanupScope {
    CleanupScope {
        shield: true,
        deadline: Instant::now()
            .checked_add(support::CALL_TIMEOUT)
            .expect("the cleanup deadline fits"),
        caller_cancelled: CancellationToken::new(),
    }
}

// The response promises a chunked body and closes after its first chunk, so the body read fails
async fn serve_truncated_stream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // the tracker keeps the server task running after the test's own call returns
    let tasks = TaskTracker::new();
    tasks.spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut head = [0_u8; 1024];
        let _read = socket.read(&mut head).await;
        let _written = socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\na\r\ndata: {}\n\n\r\n",
            )
            .await;
        let _closed = socket.shutdown().await;
    });
    format!("http://{address}/mcp")
}
