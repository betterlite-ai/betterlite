#![cfg(test)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;

use crate::refused_port::RefusedPort;

const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(200);

#[test]
fn a_connect_to_a_refused_port_is_refused() {
    let refused = RefusedPort::bind().unwrap();
    let error = TcpStream::connect_timeout(&refused.address, ATTEMPT_TIMEOUT).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
}

#[test]
fn binding_the_address_again_fails_while_the_guard_lives() {
    let refused = RefusedPort::bind().unwrap();
    assert_eq!(refused.address.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    let error = TcpListener::bind(refused.address).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
}
