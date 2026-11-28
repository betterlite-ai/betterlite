#![cfg(test)]

use betterlite_context::local_ttl_cache::CacheValue;

use super::{read_redis_copy, write_redis_copy};
use crate::mcp::oauth_sessions::tests::testing::{
    CipherChoice, HarnessConfig, RedisFault, harness, keyed_cipher, seal, server, server_named,
    temporary_key,
};

#[tokio::test]
async fn a_copy_round_trips_under_the_prefixed_key_and_ttl() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let stored = server_named("test-session-1", "from-cache");
    write_redis_copy(&harness.store, &stored, 300).await;
    let redis = harness.redis.as_ref().expect("the backend is set");
    let key = temporary_key("test-session-1");
    assert_eq!(redis.ttl(&key), Some(300.0_f64));
    let value = redis.get(&key).expect("the write stored a value");
    let CacheValue::Text(sealed) = value else {
        panic!("the copy is an encrypted string");
    };
    let plaintext = serde_json::to_string(&stored).expect("the fixture server encodes");
    assert_ne!(
        sealed, plaintext,
        "the copy is sealed, not the server record"
    );
    let read = read_redis_copy(&harness.store, "test-session-1")
        .await
        .expect("the copy reads");
    assert_eq!(read.fields.name, "from-cache");
}

#[tokio::test]
async fn a_non_positive_ttl_is_raised_to_one_second_in_redis() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    write_redis_copy(&harness.store, &server("test-session-1"), 0).await;
    let redis = harness.redis.as_ref().expect("the backend is set");
    assert_eq!(redis.ttl(&temporary_key("test-session-1")), Some(1.0_f64));
}

#[tokio::test]
async fn without_a_backend_the_copy_is_skipped() {
    let harness = harness(HarnessConfig::new());
    write_redis_copy(&harness.store, &server("test-session-1"), 300).await;
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn an_encryption_failure_stores_nothing() {
    let harness = harness(
        HarnessConfig::new()
            .with_redis(RedisFault::None)
            .with_cipher(CipherChoice::Keyless),
    );
    write_redis_copy(&harness.store, &server("test-session-1"), 300).await;
    let redis = harness.redis.as_ref().expect("the backend is set");
    assert_eq!(redis.len(), 0);
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_write_fault_is_ignored() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::Write));
    write_redis_copy(&harness.store, &server("test-session-1"), 300).await;
    let redis = harness.redis.as_ref().expect("the backend is set");
    assert_eq!(redis.len(), 0);
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_read_fault_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::Read));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = seal(
        &harness.cipher,
        &serde_json::to_string(&server("test-session-1")).expect("the fixture server encodes"),
    );
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_non_string_value_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    redis.put(&temporary_key("test-session-1"), CacheValue::Integer(7));
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn an_unencrypted_value_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let plaintext = serde_json::to_string(&server("test-session-1")).expect("the fixture encodes");
    redis.put(
        &temporary_key("test-session-1"),
        CacheValue::Text(plaintext),
    );
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_decrypted_value_that_is_not_json_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = seal(&harness.cipher, "not json");
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_decrypted_json_value_that_is_not_an_object_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = seal(&harness.cipher, "[1, 2]");
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_decrypted_object_that_is_not_a_server_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = seal(&harness.cipher, r#"{"server_id":"test-session-1"}"#);
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}

#[tokio::test]
async fn a_value_sealed_with_another_key_reads_as_a_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let other = keyed_cipher("other-salt");
    let encoded = serde_json::to_string(&server("test-session-1")).expect("the fixture encodes");
    redis.put(
        &temporary_key("test-session-1"),
        CacheValue::Text(seal(&other, &encoded)),
    );
    let read = read_redis_copy(&harness.store, "test-session-1").await;
    assert!(read.is_none());
}
