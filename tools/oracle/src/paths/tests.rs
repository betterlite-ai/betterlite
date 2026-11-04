#![cfg(test)]

use std::fs;
use std::net::TcpListener;

use super::{free_port, port_in_range};
use rstest::rstest;

#[test]
fn gateway_ports_are_below_the_ephemeral_range_and_rebindable() {
    let range = fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range").unwrap();
    let start = range
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let port = free_port().unwrap();
    assert!((10000..start).contains(&port), "gateway port {port}");
    let _listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn held_gateway_ports_are_skipped() {
    let port = free_port().unwrap();
    let _listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let next = free_port().unwrap();
    assert_ne!(next, port);
}

#[rstest]
#[case::malformed("bad", "ip_local_port_range")]
#[case::empty("", "ip_local_port_range")]
#[case::no_ports("10000 60999", "10000..10000")]
#[case::below_minimum("9999 60999", "10000..9999")]
fn invalid_ranges_never_fall_back(#[case] range: &str, #[case] diagnostic: &str) {
    let error = port_in_range(range, u32::MAX).unwrap_err();
    assert!(error.to_string().contains(diagnostic));
}

#[test]
fn no_port_in_range_binds() {
    let port = port_in_range("11000 60999", 0).unwrap();
    let _listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let error = port_in_range(&format!("{} 60999", port + 1), 0).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("no free gateway port on 127.0.0.1 in 10000..{}", port + 1)
    );
}
