#![cfg(test)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use super::serve;
use crate::error::FixtureError;
use crate::reply::reply;

#[test]
fn answers_a_client_that_connects_after_the_old_deadline() {
    let serving = serve(reply("200 fixture", &[], "ok"), Duration::ZERO).unwrap();
    thread::sleep(Duration::from_secs(6));
    let mut stream = TcpStream::connect(serving.address).unwrap();
    stream
        .write_all(b"GET /late HTTP/1.1\r\nhost: fixture\r\n\r\n")
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    assert!(answer.ends_with("ok"), "{answer}");
    assert_eq!(serving.finish().unwrap().line, "GET /late HTTP/1.1");
}

#[test]
fn finish_releases_a_fixture_the_client_never_reached() {
    let serving = serve(reply("200 fixture", &[], "ok"), Duration::ZERO).unwrap();

    assert!(matches!(serving.finish(), Err(FixtureError::NoRequest)));
}

#[test]
fn finish_returns_what_the_client_sent() {
    let body = r#"{"input":"hello"}"#;
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let mut stream = TcpStream::connect(serving.address).unwrap();
    let request = format!(
        "POST /v1beta/interactions HTTP/1.1\r\nhost: fixture\r\nx-fixture: yes\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();

    let received = serving.finish().unwrap();

    assert_eq!(received.line, "POST /v1beta/interactions HTTP/1.1");
    assert!(received.head.contains("x-fixture: yes"));
    assert_eq!(received.body, body);
}

#[test]
fn a_client_that_leaves_during_the_pause_still_finishes() {
    let serving = serve(reply("200 fixture", &[], "ok"), Duration::from_millis(200)).unwrap();
    let mut stream = TcpStream::connect(serving.address).unwrap();
    stream
        .write_all(b"GET /gone HTTP/1.1\r\nhost: fixture\r\n\r\n")
        .unwrap();
    drop(stream);

    assert_eq!(serving.finish().unwrap().line, "GET /gone HTTP/1.1");
}
