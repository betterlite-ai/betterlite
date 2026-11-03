use super::*;

#[test]
fn loopback_server_binds_127_0_0_1_and_redirects_to_its_port() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server =
        LoopbackServer::with_listener(listener, "state", CONNECTION_TIMEOUT).expect("server");
    assert_eq!(server.address.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert_ne!(server.address.port(), 0);
    assert_eq!(
        server.redirect_uri(),
        format!("http://127.0.0.1:{}/callback", server.address.port())
    );
}

#[test]
fn loopback_drops_a_slow_sender_at_the_connection_deadline() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server = LoopbackServer::with_listener(listener, "state", Duration::from_millis(150))
        .expect("server");
    let address = server.address;
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut slow = TcpStream::connect(address).expect("slow connect");
            slow.write_all(b"G").expect("first byte");
            thread::sleep(Duration::from_millis(250));
            let mut valid = TcpStream::connect(address).expect("valid connect");
            valid
                .write_all(b"GET /callback?state=state&code=good HTTP/1.1\r\n\r\n")
                .expect("valid request");
        });
        let mut callback_server = server;
        let outcome = callback_server
            .wait(Duration::from_secs(2), &SystemClock)
            .expect("valid callback");
        let CallbackOutcome::Code(code) = outcome else {
            panic!("expected code");
        };
        assert_eq!(code.code.expose_secret(), "good");
    });
}

#[test]
fn loopback_wait_deadline_cuts_off_a_slow_sender() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server =
        LoopbackServer::with_listener(listener, "state", CONNECTION_TIMEOUT).expect("server");
    let address = server.address;
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut slow = TcpStream::connect(address).expect("slow connect");
            slow.write_all(b"G").expect("first byte");
            thread::sleep(Duration::from_millis(300));
        });
        let mut callback_server = server;
        let result = callback_server.wait(Duration::from_millis(100), &SystemClock);
        assert!(matches!(result, Err(PkceFailure::CallbackTimeout)));
    });
}
