#![cfg(test)]

use std::io::{self, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::ensure_supported;
use super::redis::RedisServer;
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::PollingTransport as _;

const KEY: &str = "litellm:polling:response:litellm_poll_scenario";
const VALUE: &str = r#"{"id":"litellm_poll_scenario","object":"response","status":"queued","created_at":1700000000,"output":[],"usage":null,"metadata":{}}"#;
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
// under the fixture's 5 s idle read timeout, so a close caused only by that timeout fails the bound tests
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const OVERSIZED_HEADER_BYTES: usize = 0x0001_0001;

#[test]
fn polling_entry_is_stored_under_the_peer_prefix_with_ttl() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let mut polling = server.polling();
    polling
        .write("first", KEY, VALUE, 3600)
        .expect("the cache client stores the polling entry");
    let stored = polling
        .entry("first:litellm:polling:response:litellm_poll_scenario")
        .expect("the entry is stored under the peer prefix");
    assert_eq!(stored.value, VALUE);
    assert_eq!(stored.ttl_seconds, Some(3600));
}

#[test]
fn polling_entries_of_two_peers_do_not_share_a_key() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let mut first = server.polling();
    let mut second = server.polling();
    first
        .write("first", KEY, VALUE, 3600)
        .expect("the first peer stores its entry");
    second
        .write("second", KEY, VALUE, 3600)
        .expect("the second peer stores its entry");
    assert!(
        first
            .entry("first:litellm:polling:response:litellm_poll_scenario")
            .is_some()
    );
    assert!(
        second
            .entry("second:litellm:polling:response:litellm_poll_scenario")
            .is_some()
    );
    assert!(first.entry(KEY).is_none());
}

#[test]
fn unknown_command_is_refused_and_recorded() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let reply = raw_reply(server.addr, b"*2\r\n$4\r\nMGET\r\n$1\r\nk\r\n");
    assert!(reply.contains("-ERR unknown command 'MGET'"), "{reply}");
    assert_eq!(server.unsupported().as_deref(), Some("MGET"));
    let error = ensure_supported(&server).expect_err("the unknown command fails the scenario");
    assert!(matches!(error, ScenarioError::UnknownCommand { command } if command == "MGET"));
}

#[test]
fn unsupported_set_form_is_refused_and_recorded() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let reply = raw_reply(
        server.addr,
        b"*4\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n$2\r\nNX\r\n",
    );
    assert!(reply.contains("-ERR unknown command 'SET'"), "{reply}");
    assert_eq!(server.unsupported().as_deref(), Some("SET"));
    let error = ensure_supported(&server).expect_err("the unsupported SET form fails the scenario");
    assert!(matches!(error, ScenarioError::UnknownCommand { command } if command == "SET"));
}

#[test]
fn oversized_bulk_length_closes_the_connection() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let reply = raw_after_close(server.addr, b"*2\r\n$3\r\nGET\r\n$99999999999\r\n");
    assert!(reply.is_empty(), "the connection answered {reply:?}");
    assert!(server.unsupported().is_none());
}

#[test]
fn oversized_header_line_closes_the_connection() {
    let server = RedisServer::start().expect("the polling redis binds a loopback port");
    let request = vec![b'a'; OVERSIZED_HEADER_BYTES];
    let reply = raw_after_close(server.addr, &request);
    assert!(reply.is_empty(), "the connection answered {reply:?}");
}

fn raw_stream(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).expect("the fake redis accepts a connection");
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .expect("the write timeout is set");
    stream
}

fn raw_reply(addr: SocketAddr, request: &[u8]) -> String {
    let mut stream = raw_stream(addr);
    stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .expect("the read timeout is set");
    stream.write_all(request).expect("the command is sent");
    let mut buffer = [0_u8; 64];
    let read = stream.read(&mut buffer).expect("the server answers");
    let reply = buffer
        .get(..read)
        .expect("the read length stays in the buffer");
    String::from_utf8(reply.to_vec()).expect("the reply is UTF-8")
}

fn raw_after_close(addr: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = raw_stream(addr);
    stream
        .set_read_timeout(Some(CLOSE_TIMEOUT))
        .expect("the close timeout is set");
    let _written = stream.write_all(request);
    let mut reply = Vec::new();
    let outcome = stream.read_to_end(&mut reply);
    if let Err(error) = &outcome
        && error.kind() != io::ErrorKind::ConnectionReset
    {
        panic!("the connection stayed open: {error}");
    }
    reply
}
