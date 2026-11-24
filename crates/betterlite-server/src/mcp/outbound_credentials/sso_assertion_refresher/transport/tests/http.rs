#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_test_ports::refused_port::RefusedPort;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::super::{GrantAnswer, HttpGrantFormPost, RefreshGrantPost as _};

fn form() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("grant_type".to_owned(), "refresh_token".to_owned()),
        ("refresh_token".to_owned(), "carried-token".to_owned()),
    ])
}

fn poster(limits: TransportLimits) -> HttpGrantFormPost {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpGrantFormPost {
        client: Arc::new(ProxiedClient::build(base_builder(), &policy, limits).unwrap()),
        deadline: Duration::from_secs(5),
    }
}

fn response(status_line: &str, body: &str) -> String {
    format!(
        "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !collected.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await.expect("the request arrives");
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read]);
    }
    collected
}

async fn serve(listener: TcpListener, reply: String) -> Vec<u8> {
    let (mut stream, _peer) = listener.accept().await.expect("the client connects");
    let request = read_request(&mut stream).await;
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the response is written");
    stream.flush().await.expect("the response is flushed");
    request
}

async fn exchange(reply: String, post: HttpGrantFormPost) -> (String, GrantAnswer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(listener, reply));
    let answer = post
        .post_form(
            &format!("http://{address}/token"),
            &form(),
            &BTreeMap::new(),
        )
        .await;
    let request = tasks.join_next().await.unwrap().unwrap();
    (String::from_utf8(request).unwrap(), answer)
}

#[tokio::test]
async fn sends_the_grant_form_with_an_accept_header_and_returns_the_body() {
    let reply = response("HTTP/1.1 200 OK", "{\"id_token\":\"tok\"}");
    let (request, answer) = exchange(reply, poster(TransportLimits::DEFAULT)).await;
    assert!(request.starts_with("POST /token HTTP/1.1\r\n"));
    assert!(request.contains("accept: application/json\r\n"));
    assert!(request.contains("content-type: application/x-www-form-urlencoded\r\n"));
    let GrantAnswer::Answered { status, body } = answer else {
        panic!("a 200 is an answer");
    };
    assert_eq!(status, 200);
    assert_eq!(str::from_utf8(&body).unwrap(), "{\"id_token\":\"tok\"}");
}

#[tokio::test]
async fn a_refused_status_keeps_its_status_and_carries_no_body() {
    let reply = response("HTTP/1.1 400 Bad Request", "denied");
    let (_request, answer) = exchange(reply, poster(TransportLimits::DEFAULT)).await;
    let GrantAnswer::Answered { status, body } = answer else {
        panic!("a 400 is an answer");
    };
    assert_eq!(status, 400);
    assert!(body.is_empty());
}

#[tokio::test]
async fn a_body_over_the_pool_cap_is_unreachable() {
    let reply = response("HTTP/1.1 200 OK", "{\"id_token\":\"abcdefghijklmnop\"}");
    let limits = TransportLimits {
        body_bytes: 16,
        ..TransportLimits::DEFAULT
    };
    let (_request, answer) = exchange(reply, poster(limits)).await;
    assert!(matches!(answer, GrantAnswer::Unreachable));
}

#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let refused = RefusedPort::bind().unwrap();
    let answer = poster(TransportLimits::DEFAULT)
        .post_form(
            &format!("http://{}/token", refused.address),
            &form(),
            &BTreeMap::new(),
        )
        .await;
    assert!(matches!(answer, GrantAnswer::Unreachable));
}

#[tokio::test]
async fn an_unbuildable_header_is_unreachable() {
    let headers = BTreeMap::from([("X-Bad".to_owned(), "a\nb".to_owned())]);
    let answer = poster(TransportLimits::DEFAULT)
        .post_form("http://idp.example/token", &form(), &headers)
        .await;
    assert!(matches!(answer, GrantAnswer::Unreachable));
}

#[tokio::test]
async fn a_url_that_does_not_parse_is_unreachable() {
    let answer = poster(TransportLimits::DEFAULT)
        .post_form("not a url", &form(), &BTreeMap::new())
        .await;
    assert!(matches!(answer, GrantAnswer::Unreachable));
}
