#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::advance;

use super::secret_url;
use crate::cyberark::fake_transport::{AUTH_ROUTE, Reply, capture_logs, keyed};
use crate::cyberark::ttl_cache::TtlCache;

const SECRET_ROUTE: &str = "GET http://127.0.0.1:8080/secrets/default/variable/db";

#[rstest]
#[case::plain("db", "db")]
#[case::slash("prod/db", "prod%2Fdb")]
#[case::space("my secret", "my%20secret")]
#[case::unreserved("a.b-c_d~e", "a.b-c_d~e")]
#[case::reserved("a+b=c&d%e", "a%2Bb%3Dc%26d%25e")]
#[case::utf8("r\u{e9}sum\u{e9}", "r%C3%A9sum%C3%A9")]
fn names_are_percent_encoded_in_the_url(#[case] name: &str, #[case] encoded: &str) {
    let (manager, _transport) = keyed(&[]);
    assert_eq!(
        secret_url(&manager, name),
        format!("http://127.0.0.1:8080/secrets/default/variable/{encoded}")
    );
}

#[tokio::test(start_paused = true)]
async fn a_read_sends_the_token_and_returns_the_text() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "s3cret")])]);
    let value = manager.read("db").await.unwrap();
    assert_eq!(value.expose_secret(), "s3cret");
    let read = transport.recorded().into_iter().nth(1).unwrap();
    assert_eq!(read.route, SECRET_ROUTE);
    assert_eq!(read.body, None);
    assert_eq!(read.content_type, None);
    assert!(!read.blocking);
}

#[tokio::test(start_paused = true)]
async fn values_are_cached_for_the_refresh_interval() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "v")])]);
    manager.read("db").await;
    advance(Duration::from_secs(300)).await;
    manager.read("db").await;
    assert_eq!(transport.routes_hit().len(), 2);
    advance(Duration::from_millis(1)).await;
    manager.read("db").await;
    assert_eq!(transport.routes_hit().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn an_empty_value_is_still_a_cache_hit() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "")])]);
    assert!(manager.read("db").await.is_some());
    assert!(manager.read("db").await.is_some());
    assert_eq!(transport.routes_hit().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn not_found_is_none_and_logged_at_debug() {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(404, "")])]);
    assert!(manager.read("db").await.is_none());
    let text = logs.text();
    assert!(text.contains("DEBUG Secret db not found in CyberArk Conjur"));
    assert!(!text.contains("ERROR"));
}

#[rstest]
#[case::unauthorized(Reply::Status(401, ""))]
#[case::server_error(Reply::Status(500, ""))]
#[case::redirect(Reply::Status(302, ""))]
#[case::connection(Reply::Failure)]
#[tokio::test(start_paused = true)]
async fn other_failures_are_none_and_logged_as_errors(#[case] reply: Reply) {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) = keyed(&[(SECRET_ROUTE, vec![reply])]);
    assert!(manager.read("db").await.is_none());
    assert!(
        logs.text()
            .contains("ERROR Error reading secret from CyberArk Conjur")
    );
    assert!(manager.read("db").await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_failed_login_makes_the_read_none_with_an_error_log() {
    let (logs, _guard) = capture_logs();
    let (manager, _transport) = keyed(&[(AUTH_ROUTE, vec![Reply::Status(401, "")])]);
    assert!(manager.read("db").await.is_none());
    assert!(
        logs.text()
            .contains("ERROR Error reading secret from CyberArk Conjur: Could not authenticate")
    );
}

#[tokio::test(start_paused = true)]
async fn reads_do_not_check_the_name_for_unsafe_characters() {
    let route = "GET http://127.0.0.1:8080/secrets/default/variable/bad%20name";
    let (manager, _transport) = keyed(&[(route, vec![Reply::Status(200, "v")])]);
    assert!(manager.read("bad name").await.is_some());
}

#[test]
fn the_sync_read_uses_the_blocking_transport_and_shares_the_cache() {
    let (manager, transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(200, "v")])]);
    let first = manager.read_blocking("db").unwrap();
    let second = manager.read_blocking("db").unwrap();
    assert_eq!(first.expose_secret(), second.expose_secret());
    let seen = transport.recorded();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|request| request.blocking));
}

#[test]
fn the_sync_read_returns_none_for_a_missing_secret() {
    let (manager, _transport) = keyed(&[(SECRET_ROUTE, vec![Reply::Status(404, "")])]);
    assert!(manager.read_blocking("db").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_secret_named_like_the_token_key_never_returns_the_token() {
    let route = "GET http://127.0.0.1:8080/secrets/default/variable/cyberark_auth_token";
    let (manager, transport) = keyed(&[(route, vec![Reply::Status(404, "")])]);
    manager.authorization().await.unwrap();
    assert!(manager.read("cyberark_auth_token").await.is_none());
    assert_eq!(
        transport.routes_hit().last().map(String::as_str),
        Some(route)
    );
}

async fn filled_cache(entries: u32) -> TtlCache {
    let cache = TtlCache::new(Duration::from_secs(60));
    for index in 0..entries {
        cache.set(&format!("key{index:03}"), SecretString::from("v"));
        advance(Duration::from_millis(1)).await;
    }
    cache
}

#[tokio::test(start_paused = true)]
async fn the_cache_evicts_the_earliest_expiry_at_capacity() {
    let cache = filled_cache(200).await;
    cache.set("newest", SecretString::from("v"));
    assert!(cache.get("key000").is_none());
    assert!(cache.get("key001").is_some());
    assert!(cache.get("newest").is_some());
}

#[tokio::test(start_paused = true)]
async fn overwriting_a_live_entry_keeps_its_first_expiry() {
    let cache = TtlCache::new(Duration::from_secs(10));
    cache.set("key", SecretString::from("old"));
    advance(Duration::from_secs(6)).await;
    cache.set("key", SecretString::from("new"));
    assert_eq!(cache.get("key").unwrap().expose_secret(), "new");
    advance(Duration::from_secs(5)).await;
    assert!(cache.get("key").is_none());
}

#[tokio::test(start_paused = true)]
async fn deleting_removes_the_entry() {
    let cache = TtlCache::new(Duration::from_secs(10));
    cache.set("key", SecretString::from("v"));
    cache.delete("key");
    assert!(cache.get("key").is_none());
}
