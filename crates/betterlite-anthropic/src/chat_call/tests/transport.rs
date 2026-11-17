use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::HeaderMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;
use url::Url;

use crate::anthropic_http_stream::error::PostFailure;
use crate::anthropic_http_stream::transport::StreamPost;
use crate::chat_call::transport::ChatTransport as _;

const SENT: &[u8] = b"{\"model\":\"claude-sonnet-4-6\"}";
const ANSWER: &str = "{\"id\":\"msg_1\"}";
const DEADLINE: Duration = Duration::from_secs(10);

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

fn post_of(url: Url) -> StreamPost {
    StreamPost {
        url,
        headers: HeaderMap::new(),
        body: SENT.to_vec(),
        stream: false,
        deadline: DEADLINE,
    }
}

fn answer(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn local_url(listener: &TcpListener) -> Url {
    let address = listener.local_addr().unwrap();
    Url::parse(&format!("http://{address}/v1/messages")).unwrap()
}

async fn serve(listener: TcpListener, response: String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(response.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
    let mut rest = Vec::new();
    let _closed = socket.read_to_end(&mut rest).await;
}

async fn join(tasks: &mut JoinSet<()>) {
    timeout(DEADLINE, tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn chat_call_buffered_transport_status() {
    // a non-2xx reply is a status failure carrying the response body and headers
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = local_url(&listener);
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(listener, answer(500, ANSWER)));
    let client = proxied_client();
    let failed = timeout(DEADLINE, client.post_buffered(post_of(url)))
        .await
        .unwrap();
    let Err(PostFailure::Status(failure)) = failed else {
        panic!("a 500 reply is a status failure");
    };
    assert_eq!(failure.status_code, 500);
    assert_eq!(failure.body, ANSWER.as_bytes());
    // httpx masks a status error, so the handler reads the response headers instead
    assert!(failure.exception_headers.is_none());
    let headers = failure.response_headers.unwrap();
    assert_eq!(
        headers
            .get("content-type")
            .map(|value| value.to_str().unwrap()),
        Some("application/json")
    );
    client
        .drain(DEADLINE)
        .await
        .expect("the transport owner drains");
    join(&mut tasks).await;

    // a 2xx reply is the buffered answer itself
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = local_url(&listener);
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(listener, answer(200, ANSWER)));
    let client = proxied_client();
    let reply = timeout(DEADLINE, client.post_buffered(post_of(url)))
        .await
        .unwrap()
        .expect("a 200 reply is the buffered answer");
    assert_eq!(reply.status_code, 200);
    assert_eq!(reply.body, ANSWER.as_bytes());
    assert_eq!(
        reply
            .headers
            .get("content-type")
            .map(|value| value.to_str().unwrap()),
        Some("application/json")
    );
    client
        .drain(DEADLINE)
        .await
        .expect("the transport owner drains");
    join(&mut tasks).await;
}
