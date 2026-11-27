#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::AuthCodeStore;
use crate::byok_consent::test_support::record;
use crate::byok_token_exchange::ports::{AuthCodeLookup as _, AuthCodeRecord};

const NOW: f64 = 1_700_000_000.0;

#[test]
fn an_inserted_code_is_read_back() {
    let codes = AuthCodeStore::new();
    codes.insert("code-1".to_owned(), record(NOW + 300.0));
    let stored = codes.get("code-1").unwrap();
    assert_eq!(stored.expires_at, NOW + 300.0);
    assert_eq!(stored.api_key.expose_secret(), "sk-seed");
}

#[test]
fn a_missing_code_reads_as_none() {
    let codes = AuthCodeStore::new();
    assert!(codes.get("code-1").is_none());
}

#[test]
fn a_removed_code_is_gone() {
    let codes = AuthCodeStore::new();
    codes.insert("code-1".to_owned(), record(NOW + 300.0));
    codes.remove("code-1");
    assert_eq!(codes.count(), 0);
    assert!(codes.get("code-1").is_none());
}

#[test]
fn removing_a_missing_code_changes_nothing() {
    let codes = AuthCodeStore::new();
    codes.remove("code-1");
    assert_eq!(codes.count(), 0);
}

#[test]
fn replacing_a_code_keeps_one_entry() {
    let codes = AuthCodeStore::new();
    codes.insert("code-1".to_owned(), record(NOW + 300.0));
    codes.insert("code-1".to_owned(), record(NOW + 1.0));
    assert_eq!(codes.count(), 1);
    assert_eq!(codes.get("code-1").unwrap().expires_at, NOW + 1.0);
}

#[test]
fn the_count_tracks_the_distinct_codes() {
    let codes = AuthCodeStore::new();
    codes.insert("a".to_owned(), record(NOW));
    codes.insert("b".to_owned(), record(NOW));
    assert_eq!(codes.count(), 2);
    codes.remove("a");
    assert_eq!(codes.count(), 1);
}

#[test]
fn the_store_is_empty_when_created() {
    let codes = AuthCodeStore::new();
    assert_eq!(codes.count(), 0);
    let defaulted = AuthCodeStore::default();
    assert_eq!(defaulted.count(), 0);
}

#[test]
fn a_record_round_trips_through_the_store() {
    let codes = AuthCodeStore::new();
    let written = AuthCodeRecord {
        api_key: secrecy::SecretString::from("sk-user"),
        server_id: "srv-1".to_owned(),
        code_challenge: "challenge".to_owned(),
        redirect_uri: "http://127.0.0.1:1456/callback".to_owned(),
        client_id: "client-1".to_owned(),
        user_id: "user-1".to_owned(),
        expires_at: NOW + 300.0,
    };
    codes.insert("code-1".to_owned(), written);
    let stored = codes.get("code-1").unwrap();
    assert_eq!(stored.api_key.expose_secret(), "sk-user");
    assert_eq!(stored.server_id, "srv-1");
    assert_eq!(stored.code_challenge, "challenge");
    assert_eq!(stored.redirect_uri, "http://127.0.0.1:1456/callback");
    assert_eq!(stored.client_id, "client-1");
    assert_eq!(stored.user_id, "user-1");
    assert_eq!(stored.expires_at, NOW + 300.0);
}
