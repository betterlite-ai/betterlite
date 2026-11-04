use super::super::Clock;
use super::{
    CONNECTION_TIMEOUT, CallbackOutcome, LoopbackServer, PkceFailure, TcpLoopbackListener,
};
use secrecy::ExposeSecret as _;
use std::io::Error as IoError;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

struct AdvancingClock {
    base: Instant,
    milliseconds: AtomicU64,
}

impl AdvancingClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            milliseconds: AtomicU64::new(0),
        }
    }

    fn advance(&self) {
        self.milliseconds.fetch_add(10, Ordering::SeqCst);
    }
}

impl Clock for AdvancingClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.milliseconds.load(Ordering::SeqCst))
    }
}

#[test]
fn loopback_server_binds_127_0_0_1_and_redirects_to_its_port() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server =
        LoopbackServer::with_listener(listener, "state", CONNECTION_TIMEOUT).expect("server");
    assert_eq!(server.address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(server.address.port(), 0);
    assert_eq!(
        server.redirect_uri(),
        format!("http://127.0.0.1:{}/callback", server.address.port())
    );
}

#[test]
fn loopback_drops_a_slow_sender_at_the_connection_deadline() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server = LoopbackServer::with_listener(listener, "state", Duration::from_millis(100))
        .expect("server");
    let address = server.address;
    let clock = AdvancingClock::new();
    let sender_clock = &clock;
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut slow = TcpStream::connect(address).expect("slow connect");
            let _drip_result: Result<(), IoError> =
                b"GET /callback?state=state&code=drip HTTP/1.1\r\n\r\n"
                    .iter()
                    .try_for_each(|byte| {
                        slow.write_all(std::slice::from_ref(byte))?;
                        sender_clock.advance();
                        thread::park_timeout(Duration::from_millis(3));
                        Ok(())
                    });
            let mut valid = TcpStream::connect(address).expect("valid connect");
            valid
                .write_all(b"GET /callback?state=state&code=good HTTP/1.1\r\n\r\n")
                .expect("valid request");
        });
        let mut callback_server = server;
        let outcome = callback_server
            .wait(Duration::from_secs(2), &clock)
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
        LoopbackServer::with_listener(listener, "state", Duration::from_secs(60)).expect("server");
    let address = server.address;
    let clock = AdvancingClock::new();
    let sender_clock = &clock;
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut slow = TcpStream::connect(address).expect("slow connect");
            let _drip_result: Result<(), IoError> =
                b"GET /callback?state=state&code=drip HTTP/1.1\r\n\r\n"
                    .iter()
                    .try_for_each(|byte| {
                        slow.write_all(std::slice::from_ref(byte))?;
                        sender_clock.advance();
                        thread::park_timeout(Duration::from_millis(3));
                        Ok(())
                    });
        });
        let mut callback_server = server;
        let result = callback_server.wait(Duration::from_millis(200), &clock);
        assert!(matches!(result, Err(PkceFailure::CallbackTimeout)));
    });
}

#[test]
fn loopback_drops_an_idle_connection_after_its_timeout() {
    let listener = TcpLoopbackListener::bind().expect("listener");
    let server = LoopbackServer::with_listener(listener, "state", Duration::from_millis(200))
        .expect("server");
    let address = server.address;
    let (connected_sender, connected_receiver) = sync_channel(1);
    let (release_sender, release_receiver) = sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(move || {
            let idle = TcpStream::connect(address).expect("idle connect");
            connected_sender.send(()).expect("connected");
            let mut valid = TcpStream::connect(address).expect("valid connect");
            valid
                .write_all(b"GET /callback?state=state&code=good HTTP/1.1\r\n\r\n")
                .expect("valid request");
            release_receiver.recv().expect("release idle connection");
            drop(idle);
        });
        connected_receiver.recv().expect("idle accepted soon");
        let mut callback_server = server;
        let outcome = callback_server
            .wait(Duration::from_secs(2), &super::SystemClock)
            .expect("valid callback");
        let CallbackOutcome::Code(code) = outcome else {
            panic!("expected code");
        };
        assert_eq!(code.code.expose_secret(), "good");
        release_sender.send(()).expect("release idle stream");
    });
}
