#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_client_reuse::breaker::BreakerOpen;
use redis::Value;
use rstest::rstest;

use super::{LockCacheError, deleted_one, owner_bytes, with_cache_error};

#[rstest]
#[case::read(with_cache_error::<u8, _>(
    "lease",
    Err(RedisCacheError::Breaker(BreakerOpen { name: "sensitive" })),
))]
#[case::write(with_cache_error::<u8, _>(
    "lease",
    Err(RedisCacheValuesError::BreakerOpen(BreakerOpen { name: "sensitive" })),
))]
#[case::delete(with_cache_error::<u8, _>(
    "lease",
    Err(RedisAdminError::Breaker(BreakerOpen { name: "sensitive" })),
))]
fn preserves_cache_failures_without_logging_the_cause(#[case] outcome: Result<u8, LockCacheError>) {
    let error = outcome.unwrap_err();
    assert_eq!(error.to_string(), "the redis cache lease failed");
    assert!(error.source().unwrap().to_string().contains("sensitive"));
}

#[test]
fn preserves_successful_cache_results() {
    assert_eq!(
        with_cache_error::<u8, RedisCacheError>("lease", Ok(7)).unwrap(),
        7
    );
}

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
    deleted_one(Value::BulkString(b"two".to_vec())).unwrap_err();
}

#[test]
fn treats_a_nil_reply_as_no_deletion() {
    assert!(!deleted_one(Value::Nil).unwrap());
}

#[test]
fn refuses_a_reply_that_is_not_a_count() {
    deleted_one(Value::Okay).unwrap_err();
    deleted_one(Value::SimpleString("1".to_owned())).unwrap_err();
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
        owner_bytes(CachedValue::Number(serde_json::Number::from(7_i32))),
        None
    );
    assert_eq!(owner_bytes(CachedValue::Array(Vec::new())), None);
    assert_eq!(owner_bytes(CachedValue::Object(BTreeMap::new())), None);
}
