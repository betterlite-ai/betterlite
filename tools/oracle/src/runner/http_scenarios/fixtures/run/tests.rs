#![cfg(test)]

use super::redis::RedisServer;
use crate::http_scenarios::run::PollingTransport as _;

const KEY: &str = "litellm:polling:response:litellm_poll_scenario";
const VALUE: &str = r#"{"id":"litellm_poll_scenario","object":"response","status":"queued","created_at":1700000000,"output":[],"usage":null,"metadata":{}}"#;

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
