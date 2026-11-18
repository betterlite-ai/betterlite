#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_redis::cache_primitives::decode::CachedValue;
use redis::Value;

use super::{deleted_one, owner_bytes};

#[test]
fn counts_only_an_exact_deletion_as_one() {
    assert!(deleted_one(Value::Int(1)).unwrap());
    assert!(!deleted_one(Value::Int(0)).unwrap());
    assert!(!deleted_one(Value::Int(2)).unwrap());
}

#[test]
fn accepts_the_bulk_string_an_integer_can_arrive_as() {
    assert!(deleted_one(Value::BulkString(b"1".to_vec())).unwrap());
    assert!(!deleted_one(Value::BulkString(b"0".to_vec())).unwrap());
    assert!(deleted_one(Value::BulkString(b"two".to_vec())).is_err());
}

#[test]
fn treats_a_nil_reply_as_no_deletion() {
    assert!(!deleted_one(Value::Nil).unwrap());
}

#[test]
fn refuses_a_reply_that_is_not_a_count() {
    assert!(deleted_one(Value::Okay).is_err());
    assert!(deleted_one(Value::SimpleString("1".to_owned())).is_err());
}

#[test]
fn hands_a_stored_json_string_back_as_the_owner() {
    assert_eq!(
        owner_bytes(CachedValue::Text("d3ad".to_owned())),
        Some(b"d3ad".to_vec())
    );
}

#[test]
fn only_a_json_string_can_hold_the_owner() {
    assert_eq!(owner_bytes(CachedValue::Null), None);
    assert_eq!(owner_bytes(CachedValue::Boolean(true)), None);
    assert_eq!(
        owner_bytes(CachedValue::Number(serde_json::Number::from(7))),
        None
    );
    assert_eq!(owner_bytes(CachedValue::Array(Vec::new())), None);
    assert_eq!(owner_bytes(CachedValue::Object(BTreeMap::new())), None);
}
