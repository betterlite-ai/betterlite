#![cfg(test)]

use std::net::TcpListener;
use std::time::{Duration, Instant};

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;

use super::{HEALTH_TIMEOUT, HealthTransport as _, MAX_HEALTH_BODY, ReqwestHealth, TransportError};

const SLOW_REPLY_PAUSE: Duration = Duration::from_millis(1_200);
const SHORT_TIMEOUT: Duration = Duration::from_millis(100);
const STALLED_REPLY_PAUSE: Duration = Duration::from_millis(500);

#[test]
fn reqwest_health_sends_an_unauthenticated_get_to_the_health_path() {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec(),
        Duration::ZERO,
    )
    .unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the server answers");

    assert_eq!(body, "{}");
    let request = serving.finish().unwrap();
    assert!(
        request.line.starts_with("GET /health HTTP/1.1"),
        "{}",
        request.line
    );
    assert!(!request.head.to_ascii_lowercase().contains("authorization"));
}

#[test]
fn reqwest_health_does_not_follow_redirects() {
    let serving = serve(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 7\r\nConnection: close\r\n\r\nblocked".to_vec(), Duration::ZERO).unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the redirect answer is returned as is");

    assert_eq!(body, "blocked");
    serving.finish().unwrap();
}

#[test]
fn reqwest_health_waits_for_a_slow_response_within_its_timeout() {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec(),
        SLOW_REPLY_PAUSE,
    )
    .unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the slow answer still arrives");

    assert_eq!(body, "{}");
    serving.finish().unwrap();
}

#[test]
fn reqwest_health_reports_a_refused_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let error = transport
        .get(&format!("http://{address}/health"))
        .expect_err("nothing listens on the port");

    assert!(matches!(error, TransportError::Request(_)));
}

#[test]
fn the_health_timeout_stays_five_seconds() {
    assert_eq!(HEALTH_TIMEOUT, Duration::from_secs(5));
}

#[test]
fn reqwest_health_gives_up_when_the_reply_outlasts_the_timeout() {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec(),
        STALLED_REPLY_PAUSE,
    )
    .unwrap();
    let transport = ReqwestHealth::new(SHORT_TIMEOUT).expect("the client builds");
    let started = Instant::now();

    let error = transport
        .get(&format!("http://{}/health", serving.address))
        .expect_err("the server answers after the timeout");

    let waited = started.elapsed();
    assert!(
        matches!(&error, TransportError::Request(source) if source.is_timeout()),
        "{error:?}"
    );
    assert!(waited >= SHORT_TIMEOUT, "{waited:?}");
    assert!(waited < STALLED_REPLY_PAUSE, "{waited:?}");
    serving.finish().unwrap();
}

#[test]
fn reqwest_health_accepts_a_body_of_exactly_the_limit() {
    let answer = "a".repeat(usize::try_from(MAX_HEALTH_BODY).unwrap());
    let serving = serve(reply("200 OK", &[], &answer), Duration::ZERO).unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("a body at the limit is read in full");

    assert_eq!(u64::try_from(body.len()).unwrap(), MAX_HEALTH_BODY);
    serving.finish().unwrap();
}

#[test]
fn reqwest_health_rejects_a_body_one_byte_over_the_limit() {
    let over = "a".repeat(usize::try_from(MAX_HEALTH_BODY).unwrap() + 1);
    let serving = serve(reply("200 OK", &[], &over), Duration::ZERO).unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let error = transport
        .get(&format!("http://{}/health", serving.address))
        .expect_err("the body is over the limit");

    assert!(
        matches!(error, TransportError::BodyTooLarge { limit } if limit == MAX_HEALTH_BODY),
        "{error:?}"
    );
    serving.finish().unwrap();
}

#[test]
fn reqwest_health_refuses_a_body_that_is_not_utf8() {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n\xff\xfe".to_vec(),
        Duration::ZERO,
    )
    .unwrap();
    let transport = ReqwestHealth::new(HEALTH_TIMEOUT).expect("the client builds");

    let error = transport
        .get(&format!("http://{}/health", serving.address))
        .expect_err("the bytes are not text");

    assert!(matches!(error, TransportError::NotUtf8(_)), "{error:?}");
    serving.finish().unwrap();
}
