use super::super::Clock;
use super::fixtures::{FailingListener, RecordedConnection};
use super::{
    CONNECTION_TIMEOUT, CallbackOutcome, LoopbackServer, PkceFailure, TcpLoopbackListener,
};
use secrecy::ExposeSecret as _;
use std::collections::VecDeque;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::net::TcpStream;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

struct AdvancingClock {
    base: Instant,
    seconds: AtomicU64,
}

impl Clock for AdvancingClock {
    fn now(&self) -> Instant {
        self.base
            .checked_add(Duration::from_secs(
                self.seconds.fetch_add(1, Ordering::SeqCst),
            ))
            .expect("test clock overflow")
    }
}

fn drip_then_good_listener() -> FailingListener {
    let drip = RecordedConnection {
        reads: b"GET /callback?state=state&code=drip HTTP/1.1\r\n\r\n"
            .iter()
            .map(|byte| Ok(vec![*byte]))
            .collect(),
        write_result: Ok(()),
    };
    let good = RecordedConnection {
        reads: VecDeque::from([Ok(
            b"GET /callback?state=state&code=good HTTP/1.1\r\n\r\n".to_vec()
        )]),
        write_result: Ok(()),
    };
    FailingListener {
        address: "127.0.0.1:1".parse().expect("address"),
        attempts: Mutex::new(VecDeque::from([Ok(Some(drip)), Ok(Some(good))])),
    }
}

#[test]
fn loopback_server_binds_127_0_0_1_and_redirects_to_its_port() {
    assert_eq!(CONNECTION_TIMEOUT, Duration::from_secs(5));
    let server = LoopbackServer::bind("state", CONNECTION_TIMEOUT).expect("server");
    assert_eq!(server.address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(server.address.port(), 0);
    assert_eq!(
        server.redirect_uri(),
        format!("http://127.0.0.1:{}/callback", server.address.port())
    );
}

#[test]
fn loopback_drops_a_slow_sender_at_the_connection_deadline() {
    let mut server =
        LoopbackServer::with_listener(drip_then_good_listener(), "state", CONNECTION_TIMEOUT)
            .expect("server");
    let clock = AdvancingClock {
        base: Instant::now(),
        seconds: AtomicU64::new(0),
    };
    let outcome = server
        .wait(Duration::from_secs(60), &clock)
        .expect("valid callback");
    let CallbackOutcome::Code(code) = outcome else {
        panic!("expected code");
    };
    assert_eq!(code.code.expose_secret(), "good");
}

#[test]
fn loopback_wait_deadline_cuts_off_a_slow_sender() {
    let mut server =
        LoopbackServer::with_listener(drip_then_good_listener(), "state", Duration::from_secs(60))
            .expect("server");
    let clock = AdvancingClock {
        base: Instant::now(),
        seconds: AtomicU64::new(0),
    };
    let result = server.wait(Duration::from_secs(3), &clock);
    assert!(matches!(result, Err(PkceFailure::CallbackTimeout)));
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
        let result = callback_server.wait(Duration::from_secs(2), &super::SystemClock);
        release_sender.send(()).expect("release idle stream");
        let CallbackOutcome::Code(code) = result.expect("valid callback") else {
            panic!("expected code");
        };
        assert_eq!(code.code.expose_secret(), "good");
    });
}
