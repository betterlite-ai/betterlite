#![cfg(test)]

use crate::byok_consent::store::{AUTH_CODE_TTL_SECONDS, AuthCodeStore};
use crate::byok_consent::test_support::record;
use crate::byok_token_exchange::ports::AuthCodeLookup as _;

const NOW: f64 = 1_700_000_000.0;

fn store_with(expires_at: f64) -> AuthCodeStore {
    let codes = AuthCodeStore::new();
    codes.insert("code-1".to_owned(), record(expires_at));
    codes
}

#[test]
fn a_pending_code_lives_five_minutes() {
    assert_eq!(AUTH_CODE_TTL_SECONDS, 300);
}

#[test]
fn a_code_still_live_at_its_expiry_is_kept() {
    let codes = store_with(NOW);
    codes.purge_expired(NOW);
    assert_eq!(codes.count(), 1);
}

#[test]
fn a_code_one_instant_past_its_expiry_is_purged() {
    let codes = store_with(NOW - 0.001);
    codes.purge_expired(NOW);
    assert_eq!(codes.count(), 0);
}

#[test]
fn a_code_with_five_minutes_left_survives_the_purge() {
    let codes = store_with(NOW + 300.0);
    codes.purge_expired(NOW);
    assert_eq!(codes.count(), 1);
}

#[test]
fn purging_drops_only_the_expired_codes() {
    let codes = AuthCodeStore::new();
    codes.insert("live".to_owned(), record(NOW + 1.0));
    codes.insert("expired".to_owned(), record(NOW - 1.0));
    codes.purge_expired(NOW);
    assert_eq!(codes.count(), 1);
    assert!(codes.get("live").is_some());
    assert!(codes.get("expired").is_none());
}
