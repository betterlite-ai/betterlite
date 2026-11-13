#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::super::codec::REPLY_LIMIT;
use super::{AuthPost, AuthTransport as _, Body, HttpFailure, ProxiedTransport};

struct Captured {
    head: String,
    body: String,
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/oauth/token", listener.local_addr().unwrap());
    (listener, url)
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Captured> {
    let mut head_bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !head_bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        head_bytes.push(byte[0]);
    }
    let head = String::from_utf8(head_bytes).map_err(io::Error::other)?;
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await?;
    Ok(Captured {
        head,
        body: String::from_utf8(body).map_err(io::Error::other)?,
    })
}

async fn serve(listener: TcpListener, declared: usize, sent: &[u8]) -> io::Result<Captured> {
    let (mut stream, _) = listener.accept().await?;
    let captured = read_request(&mut stream).await?;
    let reply =
        format!("HTTP/1.1 200 OK\r\ncontent-length: {declared}\r\nconnection: close\r\n\r\n");
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(sent).await?;
    stream.shutdown().await?;
    Ok(captured)
}

#[rstest]
#[case::json(
    Body::Json(SecretString::from(r#"{"client_id":"app"}"#)),
    "application/json"
)]
#[case::form(
    Body::Form(SecretString::from("grant_type=authorization_code&code=a b&c=d")),
    "application/x-www-form-urlencoded"
)]
#[tokio::test]
async fn post_puts_each_body_on_the_wire_with_its_content_type(
    #[case] body: Body,
    #[case] content_type: &str,
) {
    let expected = match &body {
        Body::Json(text) | Body::Form(text) => text.expose_secret().to_owned(),
    };
    let (listener, url) = listen().await;
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let mut transport = ProxiedTransport::new(ProxiedClient::outbound(&policy).unwrap());
    let (captured, outcome) = tokio::join!(
        serve(listener, expected.len(), expected.as_bytes()),
        transport.post(AuthPost {
            url: url.as_str(),
            body,
        })
    );
    let received = captured.unwrap();
    assert_eq!(received.body, expected);
    assert_eq!(outcome.unwrap().status, 200);
    assert!(
        received
            .head
            .contains(&format!("content-type: {content_type}"))
    );
    assert!(received.head.starts_with("POST /oauth/token HTTP/1.1\r\n"));
}

#[tokio::test]
async fn post_refuses_a_reply_over_the_reply_limit() {
    let (listener, url) = listen().await;
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let mut transport = ProxiedTransport::new(ProxiedClient::outbound(&policy).unwrap());
    let (captured, outcome) = tokio::join!(
        serve(listener, REPLY_LIMIT + 1, b""),
        transport.post(AuthPost {
            url: url.as_str(),
            body: Body::Json(SecretString::from("{}")),
        })
    );
    assert!(matches!(
        outcome,
        Err(HttpFailure::Transport(TransportError::BodyTooLarge { limit })) if limit == REPLY_LIMIT
    ));
    assert!(
        captured
            .unwrap()
            .head
            .starts_with("POST /oauth/token HTTP/1.1\r\n")
    );
}
