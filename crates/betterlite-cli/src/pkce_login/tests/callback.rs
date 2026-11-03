use super::*;

#[test]
fn callback_settles_with_a_16_kib_cookie_header() {
    let mut request = b"GET /callback?state=s&code=c HTTP/1.1\r\nCookie: ".to_vec();
    request.extend(std::iter::repeat_n(b'x', 16 * 1024));
    request.extend_from_slice(b"\r\n\r\n");
    assert!(matches!(parse_request(&request), RequestRead::Complete(_)));
}

#[test]
fn callback_settles_with_99_header_lines() {
    let mut request = b"GET /callback?state=s&code=c HTTP/1.1\r\n".to_vec();
    request.extend(
        std::iter::repeat_n(b"X: y\r\n".as_slice(), 99)
            .flatten()
            .copied(),
    );
    request.extend_from_slice(b"\r\n");
    assert!(matches!(parse_request(&request), RequestRead::Complete(_)));
}

#[test]
fn callback_answers_414_past_a_64_kib_request_line() {
    let request = vec![b'A'; 65_537];
    assert!(matches!(parse_request(&request), RequestRead::UriTooLong));
}

#[test]
fn callback_answers_431_past_the_header_limits() {
    let mut long = b"GET /callback HTTP/1.1\r\nX: ".to_vec();
    long.extend(std::iter::repeat_n(b'x', 65_536));
    long.extend_from_slice(b"\r\n\r\n");
    let mut many = b"GET /callback HTTP/1.1\r\n".to_vec();
    many.extend(
        std::iter::repeat_n(b"X: y\r\n".as_slice(), 100)
            .flatten()
            .copied(),
    );
    many.extend_from_slice(b"\r\n");
    assert!(matches!(parse_request(&long), RequestRead::HeadersTooLarge));
    assert!(matches!(parse_request(&many), RequestRead::HeadersTooLarge));
}

#[test]
fn callback_request_limits_match_http_server() {
    let mut request_line = vec![b'G'; 65_535];
    request_line.extend_from_slice(b"\n");
    assert!(matches!(
        parse_request(&request_line),
        RequestRead::Malformed | RequestRead::UnsupportedMethod
    ));
    let mut header = b"GET /callback HTTP/1.1\r\nX: ".to_vec();
    header.extend(std::iter::repeat_n(b'x', 65_531));
    header.extend_from_slice(b"\r\n\r\n");
    assert!(matches!(parse_request(&header), RequestRead::Complete(_)));
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
            let send = |request: &str, expected_status: &str| {
                let mut stream = TcpStream::connect(address).expect("connect");
                stream.write_all(request.as_bytes()).expect("write");
                let mut response = [0_u8; 512];
                let size = stream.read(&mut response).expect("read response");
                let response =
                    String::from_utf8(response.get(..size).expect("response bytes").to_vec())
                        .expect("response text");
                sender
                    .send(response.starts_with(&format!("HTTP/1.1 {expected_status}")))
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
            let send = |state: &str, expected_status: &str| {
                let mut stream = TcpStream::connect(address).expect("connect");
                let request = format!("GET /callback?state={state}&error=access_denied&error_description=nope HTTP/1.1\r\n\r\n");
                stream.write_all(request.as_bytes()).expect("write");
                let mut response = [0_u8; 512];
                let size = stream.read(&mut response).expect("read response");
                let response = String::from_utf8(response.get(..size).expect("response bytes").to_vec()).expect("response text");
                sender.send(response.starts_with(&format!("HTTP/1.1 {expected_status}"))).expect("send result");
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
