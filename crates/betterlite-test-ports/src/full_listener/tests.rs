#![cfg(test)]

use std::io;
use std::net::TcpStream;
use std::time::Duration;

use crate::full_listener::FullListener;

#[tokio::test]
async fn a_connect_to_a_full_listener_still_waits_after_200_ms() {
    let full = FullListener::bind().unwrap();
    let error = TcpStream::connect_timeout(&full.address, Duration::from_millis(200)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}
