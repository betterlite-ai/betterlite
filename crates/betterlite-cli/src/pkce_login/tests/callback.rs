use super::{
    CONNECTION_TIMEOUT, CallbackOutcome, LoopbackServer, RequestRead, SystemClock,
    TcpLoopbackListener, parse_request,
};
use secrecy::ExposeSecret as _;
use std::io::{Read as _, Write as _};
use std::iter;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

fn send_callback_request(address: SocketAddr, request: &str, expected_status: &str) -> bool {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = [0_u8; 512];
    let size = stream.read(&mut response).expect("read response");
    let text = String::from_utf8(response.get(..size).expect("response bytes").to_vec())
        .expect("response text");
    text.starts_with(&format!("HTTP/1.1 {expected_status}"))
}

#[test]
fn callback_settles_with_a_16_kib_cookie_header() {
    let mut request = b"GET /callback?state=s&code=c HTTP/1.1\r\nCookie: ".to_vec();
    request.extend(iter::repeat_n(b'x', 16 * 1024));
    request.extend_from_slice(b"\r\n\r\n");
    assert!(matches!(parse_request(&request), RequestRead::Complete(_)));
    assert_callback_request_settles(request, "200", "c");
}

#[test]
fn callback_settles_with_99_header_lines() {
    let mut request = b"GET /callback?state=s&code=c HTTP/1.1\r\n".to_vec();
    request.extend(
        iter::repeat_n(b"X: y\r\n".as_slice(), 99)
            .flatten()
            .copied(),
    );
    request.extend_from_slice(b"\r\n");
    assert!(matches!(parse_request(&request), RequestRead::Complete(_)));
    assert_callback_request_settles(request, "200", "c");
}

fn assert_callback_request_settles(request: Vec<u8>, status: &str, expected_code: &str) {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let mut server =
        LoopbackServer::with_listener(listener, "s", CONNECTION_TIMEOUT).expect("server");
    let address = server.address;
    let (sender, receiver) = sync_channel(1);
    let outcome = thread::scope(|scope| {
        scope.spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            stream.write_all(&request).expect("write request");
            let mut response = Vec::new();
            stream.read_to_end(&mut response).expect("read response");
            sender
                .send(String::from_utf8(response).expect("response text"))
                .expect("response");
        });
        server
            .wait(Duration::from_secs(2), &SystemClock)
            .expect("callback")
    });
    assert!(
        receiver
            .recv()
            .expect("response")
            .starts_with(&format!("HTTP/1.1 {status}"))
    );
    let CallbackOutcome::Code(code) = outcome else {
        panic!("expected code");
    };
    assert_eq!(code.code.expose_secret(), expected_code);
}

#[test]
fn callback_answers_414_past_a_64_kib_request_line() {
    let request = vec![b'A'; 0x0001_0001];
    assert!(matches!(parse_request(&request), RequestRead::UriTooLong));
}

#[test]
fn callback_answers_431_past_the_header_limits() {
    let mut long = b"GET /callback HTTP/1.1\r\nX: ".to_vec();
    long.extend(iter::repeat_n(b'x', 0x0001_0000));
    long.extend_from_slice(b"\r\n\r\n");
    let mut many = b"GET /callback HTTP/1.1\r\n".to_vec();
    many.extend(
        iter::repeat_n(b"X: y\r\n".as_slice(), 100)
            .flatten()
            .copied(),
    );
    many.extend_from_slice(b"\r\n");
    assert!(matches!(parse_request(&long), RequestRead::HeadersTooLarge));
    assert!(matches!(parse_request(&many), RequestRead::HeadersTooLarge));
}

#[test]
fn callback_request_limits_match_http_server() {
    let mut request_line = vec![b'G'; 0xFFFF];
    request_line.extend_from_slice(b"\n");
    assert!(matches!(
        parse_request(&request_line),
        RequestRead::Malformed | RequestRead::UnsupportedMethod
    ));
    let mut header = b"GET /callback HTTP/1.1\r\nX: ".to_vec();
    header.extend(iter::repeat_n(b'x', 0xFFFB));
    header.extend_from_slice(b"\r\n\r\n");
    assert!(matches!(parse_request(&header), RequestRead::Complete(_)));
}

#[test]
fn callback_request_rejects_repeated_leading_blank_lines_and_later_oversized_lines() {
    let mut blank_then_long = b"\r\n\r\n".to_vec();
    blank_then_long.extend(iter::repeat_n(b'A', 0x0001_0001));
    assert!(matches!(
        parse_request(&blank_then_long),
        RequestRead::Malformed
    ));
    let mut blank_lines = Vec::new();
    blank_lines.extend(iter::repeat_n(b"\r\n".as_slice(), 4096).flatten().copied());
    assert!(matches!(
        parse_request(&blank_lines),
        RequestRead::Malformed
    ));
}

#[test]
fn callback_settles_only_on_a_code_for_the_expected_state() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let mut server =
        LoopbackServer::with_listener(listener, "state", CONNECTION_TIMEOUT).expect("server");
    let address = server.address;
    let (sender, receiver) = sync_channel(4);
    let outcome = thread::scope(|scope| {
        scope.spawn(move || {
            let send = |request, expected_status| {
                sender
                    .send(send_callback_request(address, request, expected_status))
                    .expect("send result");
            };
            send("GET /nope HTTP/1.1\r\n\r\n", "404");
            send(
                "GET /callback?state=wrong&code=stolen HTTP/1.1\r\n\r\n",
                "400",
            );
            send("GET /callback?state=state HTTP/1.1\r\n\r\n", "400");
            send(
                "GET /callback?state=state&code=the-code HTTP/1.1\r\n\r\n",
                "200",
            );
        });
        server
            .wait(Duration::from_secs(2), &SystemClock)
            .expect("code outcome")
    });
    assert!(receiver.recv().expect("wrong path response"));
    assert!(receiver.recv().expect("wrong state response"));
    assert!(receiver.recv().expect("missing code response"));
    assert!(receiver.recv().expect("matching state response"));
    let CallbackOutcome::Code(code) = outcome else {
        panic!("expected a code");
    };
    assert_eq!(code.code.expose_secret(), "the-code");
}

#[test]
fn callback_denied_settles_only_for_the_expected_state() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let mut server =
        LoopbackServer::with_listener(listener, "state", CONNECTION_TIMEOUT).expect("server");
    let address = server.address;
    let (sender, receiver) = sync_channel(2);
    let outcome = thread::scope(|scope| {
        scope.spawn(move || {
            let send = |state: &str, expected_status| {
                let request = format!(
                    "GET /callback?state={state}&error=access_denied&error_description=nope HTTP/1.1\r\n\r\n"
                );
                sender
                    .send(send_callback_request(address, &request, expected_status))
                    .expect("send result");
            };
            send("wrong", "400");
            send("state", "200");
        });
        server
            .wait(Duration::from_secs(2), &SystemClock)
            .expect("denied outcome")
    });
    assert!(receiver.recv().expect("wrong state response"));
    assert!(receiver.recv().expect("matching state response"));
    let CallbackOutcome::Denied(denied) = outcome else {
        panic!("expected denial");
    };
    assert_eq!(denied.error, "access_denied");
    assert_eq!(denied.description.as_deref(), Some("nope"));
}
