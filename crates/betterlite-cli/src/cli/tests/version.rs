use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use crate::cli::tests::support::{FakeEnvironment, Harness};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const ACCEPT_DEADLINE: Duration = Duration::from_secs(5);
const POLL_PAUSE: Duration = Duration::from_millis(5);

struct Answer {
    status: u16,
    reason: &'static str,
    body: &'static str,
}

fn serve<'scope>(scope: &'scope thread::Scope<'scope, '_>, answer: &'scope Answer) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    scope.spawn(move || accept_once(&listener, answer));
    format!("http://{address}")
}

fn accept_once(listener: &TcpListener, answer: &Answer) {
    let deadline = Instant::now().checked_add(ACCEPT_DEADLINE).unwrap();
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _peer)) => return respond(stream, answer),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::park_timeout(POLL_PAUSE);
            }
            Err(_error) => return,
        }
    }
}

fn respond(mut stream: TcpStream, answer: &Answer) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = [0_u8; 4096];
    let _read = stream.read(&mut request);
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.status,
        answer.reason,
        answer.body.len()
    );
    let _wrote_head = stream.write_all(head.as_bytes());
    let _wrote_body = stream.write_all(answer.body.as_bytes());
    let _flushed = stream.flush();
    let _shut = stream.shutdown(Shutdown::Write);
}

fn unused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

#[test]
fn version_prints_cli_url_and_server_versions() {
    let answer = Answer {
        status: 200,
        reason: "OK",
        body: "{\"litellm_version\":\"9.9.9\"}",
    };
    thread::scope(|scope| {
        let base_url = serve(scope, &answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["version", "--base-url", &base_url]);
        assert_eq!(code, 0);
        let stdout = harness.stdout();
        assert!(stdout.contains(&format!("betterlite Proxy CLI Version: {CLI_VERSION}")));
        assert!(stdout.contains(&format!("betterlite Proxy Server URL: {base_url}")));
        assert!(stdout.contains("betterlite Proxy Server Version: 9.9.9"));
        assert!(harness.stderr().is_empty());
    });
}

#[test]
fn version_flag_prints_the_cli_version_and_exits_zero() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["--version", "--base-url", &base_url]);
    assert_eq!(code, 0);
    let stdout = harness.stdout();
    assert!(stdout.starts_with(&format!("betterlite Proxy CLI Version: {CLI_VERSION}\n")));
    assert!(stdout.contains(&format!("betterlite Proxy Server URL: {base_url}")));
}

#[test]
fn short_version_flag_matches_the_long_one() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["-v", "--base-url", &base_url]);
    assert_eq!(code, 0);
    assert!(
        harness
            .stdout()
            .starts_with(&format!("betterlite Proxy CLI Version: {CLI_VERSION}\n"))
    );
}

#[test]
fn unreachable_server_version_is_reported_without_failing() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["version", "--base-url", &base_url]);
    assert_eq!(code, 0);
    assert!(
        harness
            .stdout()
            .contains("Could not retrieve server version: ")
    );
}

#[test]
fn non_success_readiness_status_is_reported_without_failing() {
    let answer = Answer {
        status: 500,
        reason: "Internal Server Error",
        body: "{}",
    };
    thread::scope(|scope| {
        let base_url = serve(scope, &answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["version", "--base-url", &base_url]);
        assert_eq!(code, 0);
        assert!(harness.stdout().contains("the proxy answered HTTP 500"));
    });
}

#[test]
fn missing_server_version_prints_unavailable() {
    let answer = Answer {
        status: 200,
        reason: "OK",
        body: "{}",
    };
    thread::scope(|scope| {
        let base_url = serve(scope, &answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["version", "--base-url", &base_url]);
        assert_eq!(code, 0);
        assert!(
            harness
                .stdout()
                .contains("betterlite Proxy Server Version: (unavailable)")
        );
    });
}
