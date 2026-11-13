#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::{ApiKeyTransport as _, Get};
use crate::copilot::device_auth::transport::{HttpTransport, TransportFailure};
use crate::openai_base_config::Headers;

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/copilot_internal/v2/token",
        listener.local_addr().unwrap()
    );
    (listener, url)
}

async fn capture(listener: TcpListener, status: &str, body: &str) -> io::Result<String> {
    let (mut stream, _) = listener.accept().await?;
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    String::from_utf8(head).map_err(io::Error::other)
}

fn transport() -> HttpTransport {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpTransport {
        client: ProxiedClient::outbound(&policy).unwrap(),
        deadline: Duration::from_secs(5),
    }
}

fn headers() -> Headers {
    Headers::from([(String::from("authorization"), String::from("token fixture"))])
}

#[tokio::test]
async fn a_key_request_goes_out_as_a_get_without_a_body() {
    let (listener, url) = listen().await;
    let mut transport = transport();
    let (captured, outcome) = tokio::join!(
        capture(listener, "200 OK", "{}"),
        transport.get(Get {
            url: url.as_str(),
            headers: headers(),
        })
    );

    let head = captured.unwrap();
    assert!(head.starts_with("GET /copilot_internal/v2/token HTTP/1.1\r\n"));
    assert!(head.contains("\r\nauthorization: token fixture\r\n"));
    let reply = outcome.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, b"{}");
}

#[tokio::test]
async fn a_failed_status_reaches_the_caller_as_a_reply() {
    let (listener, url) = listen().await;
    let mut transport = transport();
    let (captured, outcome) = tokio::join!(
        capture(listener, "500 Internal Server Error", "busy"),
        transport.get(Get {
            url: url.as_str(),
            headers: headers(),
        })
    );

    captured.unwrap();
    let reply = outcome.unwrap();
    assert_eq!(reply.status, 500);
    assert_eq!(reply.body, b"busy");
}

#[tokio::test]
async fn a_header_a_value_cannot_carry_is_refused_before_the_request() {
    let mut transport = transport();

    let outcome = transport
        .get(Get {
            url: "http://127.0.0.1:1/key",
            headers: Headers::from([(String::from("authorization"), String::from("token a\nb"))]),
        })
        .await;

    assert!(matches!(
        outcome,
        Err(TransportFailure::HeaderValue { name, .. }) if name == "authorization"
    ));
}

#[tokio::test]
async fn a_url_that_does_not_parse_is_refused() {
    let mut transport = transport();

    let outcome = transport
        .get(Get {
            url: "not a url",
            headers: headers(),
        })
        .await;

    assert!(matches!(outcome, Err(TransportFailure::Url(_))));
}
