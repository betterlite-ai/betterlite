#![cfg(test)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

use super::{HealthTransport as _, ReqwestHealth, TransportError};

struct Serving {
    address: SocketAddr,
    request: thread::JoinHandle<String>,
}

const SLOW_REPLY_PAUSE: Duration = Duration::from_millis(1_200);

fn serve_once(reply: &'static [u8], pause: Option<Duration>) -> Serving {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request = thread::Builder::new()
        .spawn(move || {
            let (mut stream, _peer) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut received = Vec::new();
            while !received.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                received.extend_from_slice(&byte);
            }
            if let Some(delay) = pause {
                let started = Instant::now();
                while started.elapsed() < delay {
                    thread::park_timeout(delay.saturating_sub(started.elapsed()));
                }
            }
            stream.write_all(reply).unwrap();
            String::from_utf8(received).unwrap()
        })
        .unwrap();
    Serving { address, request }
}

#[test]
fn reqwest_health_sends_an_unauthenticated_get_to_the_health_path() {
    let serving = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        None,
    );
    let transport = ReqwestHealth::new().expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the server answers");

    assert_eq!(body, "{}");
    let request = serving.request.join().unwrap();
    assert!(request.starts_with("GET /health HTTP/1.1"), "{request}");
    assert!(!request.to_ascii_lowercase().contains("authorization"));
}

#[test]
fn reqwest_health_does_not_follow_redirects() {
    let serving = serve_once(
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 7\r\nConnection: close\r\n\r\nblocked",
        None,
    );
    let transport = ReqwestHealth::new().expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the redirect answer is returned as is");

    assert_eq!(body, "blocked");
    serving.request.join().unwrap();
}

#[test]
fn reqwest_health_waits_for_a_slow_response_within_its_timeout() {
    let serving = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        Some(SLOW_REPLY_PAUSE),
    );
    let transport = ReqwestHealth::new().expect("the client builds");

    let body = transport
        .get(&format!("http://{}/health", serving.address))
        .expect("the slow answer still arrives");

    assert_eq!(body, "{}");
    serving.request.join().unwrap();
}

#[test]
fn reqwest_health_reports_a_refused_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let transport = ReqwestHealth::new().expect("the client builds");

    let error = transport
        .get(&format!("http://{address}/health"))
        .expect_err("nothing listens on the port");

    assert!(matches!(error, TransportError::Request(_)));
}
