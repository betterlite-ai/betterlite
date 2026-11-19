#![cfg(test)]

use std::collections::BTreeMap;
use std::future::poll_fn;
use std::io::Error as IoError;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use aws_smithy_runtime_api::client::http::HttpConnector as _;
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_types::body::SdkBody;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use bytes::Bytes;
use h2::server::{SendResponse, handshake};
use http::{HeaderValue, Response as HeadResponse};
use http_body::{Body as HttpBody, Frame};
use reqwest::Url;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::AwsDuplexClient;
use super::body::RequestBody;
use crate::proxy::limits::TransportLimits;
use crate::proxy::transport_error::TransportError;
use crate::proxy::{ProxiedClient, base_builder};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

struct ChannelBody {
    chunks: mpsc::Receiver<Bytes>,
}

impl HttpBody for ChannelBody {
    type Data = Bytes;
    type Error = IoError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.get_mut().chunks.poll_recv(cx) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn listen() -> (TcpListener, Url) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = Url::parse(&format!("http://{address}/")).unwrap();
    (listener, url)
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(
        base_builder().http2_prior_knowledge(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .unwrap()
}

fn channel_body(capacity: usize) -> (mpsc::Sender<Bytes>, SdkBody) {
    let (sender, receiver) = mpsc::channel(capacity);
    (
        sender,
        SdkBody::from_body_1_x(ChannelBody { chunks: receiver }),
    )
}

fn post(url: &Url, body: SdkBody) -> HttpRequest {
    let mut request = HttpRequest::new(body);
    request.set_method("POST").unwrap();
    request.set_uri(url.as_str()).unwrap();
    let _previous = request
        .headers_mut()
        .try_insert("content-type", "application/vnd.amazon.eventstream")
        .unwrap();
    request
}

async fn next_frame(body: &mut SdkBody) -> Option<Frame<Bytes>> {
    poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx))
        .await
        .transpose()
        .unwrap()
}

async fn next_chunk(body: &mut SdkBody) -> Bytes {
    next_frame(body).await.unwrap().into_data().unwrap()
}

#[test]
fn request_body_keeps_the_inner_length_and_end() {
    let sized = RequestBody {
        body: SdkBody::from("hello"),
        _io: tokio_util::task::TaskTracker::new().token(),
    };
    assert!(!sized.is_end_stream(), "a non-empty body is not ended");
    assert_eq!(sized.size_hint().exact(), Some(5));
    let empty = RequestBody {
        body: SdkBody::empty(),
        _io: tokio_util::task::TaskTracker::new().token(),
    };
    assert!(empty.is_end_stream(), "an empty body is ended");
    assert_eq!(empty.size_hint().exact(), Some(0));
}

#[derive(Clone, Copy)]
enum Peer {
    Interleaved,
    Cancel,
}

async fn serve(listener: TcpListener, report: mpsc::Sender<String>, mode: Peer) {
    let (socket, _) = listener.accept().await.unwrap();
    let mut connection = handshake(socket).await.unwrap();
    let mut streams = JoinSet::new();
    while let Some(result) = connection.accept().await {
        let Ok((request, respond)) = result else {
            break;
        };
        let events = report.clone();
        streams.spawn(async move { relay(request, respond, events, mode).await });
    }
    while let Some(_joined) = streams.join_next().await {}
}

async fn relay(
    request: http::Request<h2::RecvStream>,
    mut respond: SendResponse<Bytes>,
    report: mpsc::Sender<String>,
    mode: Peer,
) {
    let mut recv = request.into_body();
    let first = recv.data().await.unwrap().unwrap();
    let mut head = HeadResponse::new(());
    let _replaced = head
        .headers_mut()
        .insert("x-first", HeaderValue::from_static("yes"));
    let mut send = respond.send_response(head, false).unwrap();
    send.send_data(Bytes::from_static(b"one"), false).unwrap();
    let text = match mode {
        Peer::Interleaved => {
            let second = recv.data().await.unwrap().unwrap();
            send.send_data(Bytes::from_static(b"two"), true).unwrap();
            format!(
                "{}|{}",
                String::from_utf8(first.to_vec()).unwrap(),
                String::from_utf8(second.to_vec()).unwrap()
            )
        }
        Peer::Cancel => {
            let closed = match recv.data().await {
                Some(Ok(_)) => "data",
                Some(Err(_)) => "reset",
                None => "end",
            };
            format!("{}|{closed}", String::from_utf8(first.to_vec()).unwrap())
        }
    };
    let _sent = report.send(text).await;
}

#[tokio::test]
async fn aws_duplex_interleaved_events() {
    let (listener, url) = listen().await;
    let (report, mut reports) = mpsc::channel(1);
    let mut server = JoinSet::new();
    server.spawn(serve(listener, report, Peer::Interleaved));
    let proxied = client();
    let connector = AwsDuplexClient::from_proxied(&proxied).connector();
    let (sender, body) = channel_body(4);
    sender.send(Bytes::from_static(b"hello")).await.unwrap();
    let response = timeout(TEST_TIMEOUT, connector.call(post(&url, body)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.headers().get("x-first"), Some("yes"));
    let mut answer = response.into_body();
    assert_eq!(next_chunk(&mut answer).await, Bytes::from_static(b"one"));
    sender.send(Bytes::from_static(b"world")).await.unwrap();
    drop(sender);
    assert_eq!(next_chunk(&mut answer).await, Bytes::from_static(b"two"));
    assert!(next_frame(&mut answer).await.is_none());
    let received = timeout(TEST_TIMEOUT, reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, "hello|world");
    server.abort_all();
}

#[tokio::test]
async fn aws_duplex_cancel_reaps_io() {
    let (listener, url) = listen().await;
    let (report, mut reports) = mpsc::channel(1);
    let mut server = JoinSet::new();
    server.spawn(serve(listener, report, Peer::Cancel));
    let proxied = client();
    let connector = AwsDuplexClient::from_proxied(&proxied).connector();
    let (sender, body) = channel_body(4);
    sender.send(Bytes::from_static(b"hello")).await.unwrap();
    let response = timeout(TEST_TIMEOUT, connector.call(post(&url, body)))
        .await
        .unwrap()
        .unwrap();
    let mut answer = response.into_body();
    assert_eq!(next_chunk(&mut answer).await, Bytes::from_static(b"one"));
    let waiting = proxied.drain(Duration::from_millis(100)).await.unwrap_err();
    assert!(matches!(waiting, TransportError::DrainTimeout { .. }));
    drop(answer);
    drop(sender);
    timeout(TEST_TIMEOUT, proxied.drain(DRAIN_TIMEOUT))
        .await
        .unwrap()
        .unwrap();
    let received = timeout(TEST_TIMEOUT, reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        received == "hello|end" || received == "hello|reset",
        "the request direction stayed open: {received}"
    );
    server.abort_all();
}
