use std::collections::BTreeMap;
use std::future::pending;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::mavvrik_delivery::http_transport::{
    DEFAULT_BODY_LIMIT, HttpTransport, HttpTransportError,
};
use betterlite_logging::mavvrik_delivery::transport::{
    Method, Transport as _, TransportRequest, TransportResponse,
};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;

const SHORT_DEADLINE: Duration = Duration::from_millis(300);
const ROOMY_DEADLINE: Duration = Duration::from_secs(10);
const TEST_PATIENCE: Duration = Duration::from_secs(20);
const EMPTY_OK: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const UNREACHABLE: &str = "http://127.0.0.1:1/";

fn transport(body_limit: usize) -> HttpTransport {
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let tls = TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    };
    let pool = ClientPool::new(1, TransportLimits::DEFAULT);
    HttpTransport {
        client: pool.client(&tls, &routing).unwrap(),
        body_limit,
    }
}

fn request(
    method: Method,
    url: String,
    api_key: Option<&str>,
    deadline: Duration,
) -> TransportRequest {
    TransportRequest {
        method,
        url,
        headers: Vec::new(),
        api_key: api_key.map(SecretString::from),
        body: Vec::new(),
        timeout: deadline,
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut byte = [0_u8; 1];
    while !received.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        received.push(byte[0]);
    }
    let head = from_utf8(&received).unwrap().to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    received.extend_from_slice(&body);
    String::from_utf8(received).unwrap()
}

// Answers one request with `reply`; with `hold` it then keeps the connection open and silent
async fn serve(listener: TcpListener, reply: &'static [u8], hold: bool) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let received = read_request(&mut stream).await;
    stream.write_all(reply).await.unwrap();
    if hold {
        pending::<()>().await;
    }
    received
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

#[rstest]
#[case::register_post(Method::Post, "POST", Some("key-123"))]
#[case::upload_url_get(Method::Get, "GET", Some("key-123"))]
#[case::marker_patch(Method::Patch, "PATCH", Some("key-123"))]
#[case::gcs_session_post(Method::Post, "POST", None)]
#[case::gcs_chunk_put(Method::Put, "PUT", None)]
#[case::gcs_cancel_delete(Method::Delete, "DELETE", None)]
#[tokio::test]
async fn the_api_key_goes_out_as_x_api_key_only_when_the_request_carries_one(
    #[case] method: Method,
    #[case] verb: &str,
    #[case] api_key: Option<&str>,
) {
    let (listener, base) = listen().await;
    let call = request(method, format!("{base}/path"), api_key, ROOMY_DEADLINE);
    let client = transport(DEFAULT_BODY_LIMIT);
    let sending = timeout(TEST_PATIENCE, client.send(call));
    let (response, received) = tokio::join!(sending, serve(listener, EMPTY_OK, false));
    assert_eq!(response.unwrap().unwrap().status, 200);
    assert!(
        received.starts_with(&format!("{verb} /path HTTP/1.1\r\n")),
        "{received}"
    );
    assert_eq!(
        received.contains("\r\nx-api-key: key-123\r\n"),
        api_key.is_some(),
        "{received}"
    );
    assert_eq!(
        received.matches("x-api-key").count(),
        usize::from(api_key.is_some()),
        "{received}"
    );
}

#[tokio::test]
async fn the_request_goes_out_with_its_headers_and_body() {
    let (listener, base) = listen().await;
    let mut call = request(
        Method::Put,
        format!("{base}/upload/session-1"),
        None,
        ROOMY_DEADLINE,
    );
    call.headers = vec![
        ("Content-Type", "application/gzip".to_owned()),
        ("Content-Range", "bytes 0-4/*".to_owned()),
    ];
    call.body = b"chunk".to_vec();
    let client = transport(DEFAULT_BODY_LIMIT);
    let sending = client.send(call);
    let (response, received) = tokio::join!(sending, serve(listener, EMPTY_OK, false));
    response.unwrap();
    let (head, body) = received.split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("PUT /upload/session-1 HTTP/1.1\r\n"),
        "{head}"
    );
    assert!(
        head.contains("\r\ncontent-range: bytes 0-4/*\r\n"),
        "{head}"
    );
    assert!(
        head.contains("\r\ncontent-type: application/gzip\r\n"),
        "{head}"
    );
    assert_eq!(body, "chunk");
}

#[tokio::test]
async fn the_status_location_header_and_body_come_back() {
    let (listener, base) = listen().await;
    let reply = b"HTTP/1.1 201 Created\r\nLocation: https://storage.googleapis.com/upload/session-1\r\ncontent-length: 4\r\nconnection: close\r\n\r\nbody";
    let call = request(Method::Post, format!("{base}/"), None, ROOMY_DEADLINE);
    let client = transport(DEFAULT_BODY_LIMIT);
    let sending = client.send(call);
    let (sent, _) = tokio::join!(sending, serve(listener, reply, false));
    let response: TransportResponse = sent.unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(
        response.header("location"),
        Some("https://storage.googleapis.com/upload/session-1")
    );
    assert_eq!(response.body, b"body");
}

#[tokio::test]
async fn a_308_resume_incomplete_is_returned_and_not_followed() {
    let (listener, base) = listen().await;
    let reply = b"HTTP/1.1 308 Resume Incomplete\r\nrange: bytes=0-4\r\nlocation: http://127.0.0.1:1/elsewhere\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    let call = request(Method::Put, format!("{base}/"), None, ROOMY_DEADLINE);
    let client = transport(DEFAULT_BODY_LIMIT);
    let sending = client.send(call);
    let (sent, _) = tokio::join!(sending, serve(listener, reply, false));
    let response = sent.unwrap();
    assert_eq!(response.status, 308);
    assert_eq!(response.header("range"), Some("bytes=0-4"));
}

#[rstest]
#[case::no_answer_at_all(b"")]
#[case::body_cut_short(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\npartial")]
#[tokio::test]
async fn a_server_that_holds_the_answer_past_the_deadline_ends_in_the_deadline_error(
    #[case] reply: &'static [u8],
) {
    let (listener, base) = listen().await;
    let mut server = JoinSet::new();
    server.spawn(serve(listener, reply, true));
    let call = request(Method::Get, format!("{base}/"), Some("key"), SHORT_DEADLINE);
    let outcome = timeout(TEST_PATIENCE, transport(DEFAULT_BODY_LIMIT).send(call))
        .await
        .unwrap();
    match outcome {
        Err(HttpTransportError::Send(TransportError::Deadline(_))) => {}
        other => panic!("the held answer was not ended by the request deadline: {other:?}"),
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_response_body_past_the_limit_is_an_error() {
    let (listener, base) = listen().await;
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\nconnection: close\r\n\r\nhello world";
    let call = request(Method::Get, format!("{base}/"), None, ROOMY_DEADLINE);
    let client = transport(10);
    let sending = client.send(call);
    let (outcome, _) = tokio::join!(sending, serve(listener, reply, false));
    match outcome {
        Err(HttpTransportError::Send(TransportError::BodyTooLarge { limit: 10 })) => {}
        other => panic!("the oversized body was not refused: {other:?}"),
    }
}

#[tokio::test]
async fn a_url_that_does_not_parse_is_an_error_before_anything_is_sent() {
    let call = request(Method::Get, "not a url".to_owned(), None, SHORT_DEADLINE);
    let outcome = transport(DEFAULT_BODY_LIMIT).send(call).await;
    assert!(
        matches!(outcome, Err(HttpTransportError::Url(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_api_key_a_header_cannot_carry_is_an_error() {
    let call = request(
        Method::Get,
        UNREACHABLE.to_owned(),
        Some("bad\nkey"),
        SHORT_DEADLINE,
    );
    let outcome = transport(DEFAULT_BODY_LIMIT).send(call).await;
    assert!(
        matches!(outcome, Err(HttpTransportError::ApiKey(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_header_name_that_cannot_be_sent_is_an_error_naming_it() {
    let mut call = request(Method::Get, UNREACHABLE.to_owned(), None, SHORT_DEADLINE);
    call.headers = vec![("bad name", "value".to_owned())];
    let outcome = transport(DEFAULT_BODY_LIMIT).send(call).await;
    assert!(
        matches!(
            outcome,
            Err(HttpTransportError::HeaderName {
                name: "bad name",
                ..
            })
        ),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_header_value_that_cannot_be_sent_is_an_error_naming_it() {
    let mut call = request(Method::Get, UNREACHABLE.to_owned(), None, SHORT_DEADLINE);
    call.headers = vec![("x-test", "bad\nvalue".to_owned())];
    let outcome = transport(DEFAULT_BODY_LIMIT).send(call).await;
    assert!(
        matches!(
            outcome,
            Err(HttpTransportError::HeaderValue { name: "x-test", .. })
        ),
        "{outcome:?}"
    );
}
