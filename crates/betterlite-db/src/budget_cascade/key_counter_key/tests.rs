#![cfg(test)]

use super::super::linked_rows::{KeyRow, LinkedKeys};
use crate::key_hash::KeyHash;

fn row() -> KeyRow {
    KeyRow {
        token: KeyHash::from("sk-token-a"),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn key_counter_key_wraps_the_token_under_spend_key() {
    assert_eq!(row().counter_key(), "spend:key:sk-token-a".to_owned());
}

#[test]
fn key_cache_keys_are_the_bare_token() {
    assert_eq!(row().cache_keys(), vec!["sk-token-a".to_owned()]);
}
