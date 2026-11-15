#![cfg(test)]

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use tokio::io::AsyncReadExt as _;
use tokio::net::{TcpListener, TcpSocket};
use url::Url;

use crate::gemma_call::error::FailureStatus;

#[test]
fn a_handler_timeout_reports_408() {
    assert_eq!(HttpError::Timeout.status_code(), Some(408));
    assert_eq!(HttpError::TooLarge { limit: 1024 }.status_code(), None);
}

#[tokio::test]
async fn a_pooled_timeout_reports_408() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let address = listener.local_addr().expect("the listener address");
    // the listener takes the request and never answers, so the deadline on the request is what ends the call
    tokio::spawn(async move {
        let Ok((mut connection, _peer)) = listener.accept().await else {
            return;
        };
        let mut buffer = [0u8; 512];
        while matches!(connection.read(&mut buffer).await, Ok(read) if read > 0) {}
    });
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the default policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("an outbound client");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/gemma")).expect("a loopback URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_millis(200),
    };
    let error = client
        .send_buffered(request)
        .await
        .expect_err("the stalling listener never answers");
    assert!(
        matches!(
            &error,
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
        ),
        "{error:?}"
    );
    assert_eq!(error.status_code(), Some(408));
    assert_eq!(TransportError::Closed.status_code(), None);
}

#[tokio::test]
async fn a_pooled_connect_timeout_reports_408() {
    let socket = TcpSocket::new_v4().expect("a socket");
    socket
        .bind("127.0.0.1:0".parse().expect("a loopback address"))
        .expect("a bound socket");
    let listener = socket.listen(1).expect("a listening socket");
    let address = listener.local_addr().expect("the listener address");
    // the accept queue stays full, so the client's connect hangs until its connect timeout ends the call
    let _fillers: Vec<TcpStream> = (0_u8..4)
        .filter_map(|_| TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok())
        .collect();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the default policy"),
    );
    let limits = TransportLimits {
        connect_timeout: Duration::from_millis(300),
        ..TransportLimits::DEFAULT
    };
    let client = ProxiedClient::build(base_builder(), &policy, limits).expect("an outbound client");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/gemma")).expect("a loopback URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    };
    let error = client
        .send_buffered(request)
        .await
        .expect_err("the saturated listener never takes the connection");
    assert!(
        matches!(&error, TransportError::ConnectTimeout(_)),
        "{error:?}"
    );
    assert_eq!(error.status_code(), Some(408));
}
